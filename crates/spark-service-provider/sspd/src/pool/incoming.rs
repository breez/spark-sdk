//! Admitting claimed leaves to the pool. Renewing a leaf's refund timelock takes a
//! round trip to the operators, so it happens here rather than during the claim.

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use spark::services::RenewalCandidate;
use spark::signer::LeafSigningKey;
use spark::tree::{LeafPedigree, TreeNode, TreeNodeStatus, TreeService, TreeStore};

use crate::leaves::{IncomingLeafStore, is_fit};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct AdmissionDeps {
    pub store: Arc<dyn IncomingLeafStore>,
    pub tree_service: Arc<dyn TreeService>,
    pub tree_store: Arc<dyn TreeStore>,
    pub timelocks: Arc<spark::services::TimelockManager>,
    pub identity: PublicKey,
}

/// Bounded so a large backlog is renewed in batches rather than all at once.
const ADMISSION_BATCH: i64 = 64;

/// Failed attempts after which a held leaf is warned about. Not a limit: it keeps
/// being retried.
const STUCK_AFTER_ATTEMPTS: i32 = 5;

/// Renews and admits one batch of held leaves, returning whether it admitted or
/// forgot any.
pub async fn admit_once(deps: &AdmissionDeps) -> Result<bool, BoxError> {
    let held = deps.store.held(ADMISSION_BATCH).await?;
    if held.is_empty() {
        return Ok(false);
    }
    for entry in held.iter().filter(|e| e.attempts >= STUCK_AFTER_ATTEMPTS) {
        tracing::warn!(
            leaf_id = %entry.leaf.id,
            attempts = entry.attempts,
            "a claimed leaf has failed admission repeatedly and is still held",
        );
    }

    // A leaf that joined the pool on an earlier pass but was not forgotten here may
    // have been spent since: only leaves the operators still report as ours go in.
    let ids: Vec<_> = held.iter().map(|e| e.leaf.id.clone()).collect();
    let ours: Vec<TreeNode> = deps
        .tree_service
        .fetch_nodes(&ids, false)
        .await?
        .into_iter()
        .filter(|node| node.owner_identity_public_key == Some(deps.identity))
        .collect();
    let gone: Vec<_> = ids
        .into_iter()
        .filter(|id| !ours.iter().any(|node| &node.id == id))
        .collect();
    deps.store.admitted(&gone).await?;
    let (available, busy): (Vec<_>, Vec<_>) = ours
        .into_iter()
        .partition(|node| node.status == TreeNodeStatus::Available);
    let busy: Vec<_> = busy.into_iter().map(|node| node.id).collect();
    deps.store.failed(&busy).await?;

    let mut admit = Vec::new();
    let mut renew = Vec::new();
    for leaf in available {
        let pedigree = LeafPedigree {
            leaf,
            ancestors: Vec::new(),
        };
        if is_fit(&pedigree.leaf)? {
            admit.push(pedigree);
        } else {
            // A claim leaves the leaf under the key derived from its node id.
            renew.push(RenewalCandidate {
                signing_key: LeafSigningKey {
                    derived_from: pedigree.leaf.id.clone(),
                },
                pedigree,
            });
        }
    }
    if !renew.is_empty() {
        let ids: Vec<_> = renew.iter().map(|c| c.pedigree.leaf.id.clone()).collect();
        match deps.timelocks.check_renew_nodes(renew).await {
            Ok(checked) => {
                // A leaf the operators would not renew comes back unchanged and stays held.
                let (fit, unfit): (Vec<_>, Vec<_>) = checked
                    .into_iter()
                    .partition(|p| is_fit(&p.leaf).unwrap_or(false));
                let unfit: Vec<_> = unfit.iter().map(|p| p.leaf.id.clone()).collect();
                deps.store.failed(&unfit).await?;
                admit.extend(fit);
            }
            Err(e) => {
                tracing::warn!("could not renew {} claimed leaves: {e:?}", ids.len());
                deps.store.failed(&ids).await?;
            }
        }
    }
    if admit.is_empty() {
        return Ok(!gone.is_empty());
    }

    // Leaves first: the store skips a chain whose leaf it does not hold.
    let leaves: Vec<_> = admit.iter().map(|p| p.leaf.clone()).collect();
    deps.tree_store.add_leaves(&leaves).await?;
    let rooted: Vec<_> = admit
        .into_iter()
        .filter(|p| spark::tree::chain_reaches_root(&p.leaf, &p.ancestors))
        .collect();
    if !rooted.is_empty() {
        deps.tree_store.store_ancestors(&rooted).await?;
    }

    // Forgotten only once the leaf store holds them, so after a crash in between
    // they are admitted again.
    let admitted: Vec<_> = leaves.iter().map(|l| l.id.clone()).collect();
    deps.store.admitted(&admitted).await?;
    tracing::info!("{} claimed leaves joined the pool", admitted.len());
    Ok(true)
}
