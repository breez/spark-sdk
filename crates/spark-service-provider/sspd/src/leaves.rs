use std::collections::HashMap;

use bitcoin::secp256k1::{PublicKey, Secp256k1};
use spark::operator::OperatorPool;
use spark::services::{ClaimTransferConfig, ServiceError, Transfer, TransferId, TransferService};
use spark::signer::{SecretSource, Signer};
use spark::tree::{LeavesReservationId, TreeNode, TreeNodeId, TreeServiceError, TreeStore};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Which id derives a pool leaf's signing key. A deposit-tree leaf signs under the
/// leaf id the SSP picked, not the node id the operators assigned, until the SSP
/// claims the leaf from a transfer.
#[async_trait::async_trait]
pub trait LeafSigningKeys: Send + Sync {
    /// The id `node_id`'s signing key derives from, or `None` when that is
    /// `node_id` itself.
    async fn get_signing_leaf_id(&self, node_id: &str) -> Result<Option<String>, String>;

    async fn mark_signing_under_node_id(&self, node_ids: &[String]) -> Result<(), String>;
}

pub struct IncomingTransfer {
    pub transfer: Transfer,
    held_under: HashMap<TreeNodeId, PublicKey>,
}

impl IncomingTransfer {
    pub async fn query(
        operator_pool: &OperatorPool,
        signer: &dyn Signer,
        network: spark::Network,
        transfer_id: &TransferId,
    ) -> Result<Option<Self>, ServiceError> {
        use spark::operator::rpc::spark::{TransferFilter, transfer_filter::Participant};

        let identity_public_key = spark::signer::derive_identity_public_key(signer).await?;
        let network: spark::operator::rpc::spark::Network = network.into();
        let response = operator_pool
            .get_coordinator()
            .client
            .query_all_transfers(TransferFilter {
                transfer_ids: vec![transfer_id.to_string()],
                participant: Some(Participant::SenderOrReceiverIdentityPublicKey(
                    identity_public_key.serialize().to_vec(),
                )),
                network: network as i32,
                ..Default::default()
            })
            .await?;
        let Some(proto) = response.transfers.into_iter().next() else {
            return Ok(None);
        };
        let key = |bytes: &[u8]| PublicKey::from_slice(bytes).ok();
        let held_under = proto
            .leaves
            .iter()
            .filter_map(|leaf| {
                let node = leaf.leaf.as_ref()?;
                let held_under = held_under_after_tweak(
                    &key(&node.verifying_public_key)?,
                    &key(&node.signing_keyshare.as_ref()?.public_key)?,
                    &key(&leaf.pending_key_tweak_public_key)?,
                )?;
                Some((node.id.parse().ok()?, held_under))
            })
            .collect();
        Ok(Some(Self {
            transfer: Transfer::try_from(proto)?,
            held_under,
        }))
    }

    /// Whether every leaf's secret cipher is signed by the sender and holds the key
    /// the leaf is held under once the pending tweak applies. The operators cannot
    /// read the cipher, so they do not catch one holding another key.
    pub async fn is_claimable(
        &self,
        transfer_service: &TransferService,
        signer: &dyn Signer,
    ) -> bool {
        let Ok(secrets) = transfer_service
            .verify_pending_transfer(&self.transfer)
            .await
        else {
            return false;
        };
        for leaf in &self.transfer.leaves {
            let (Some(secret), Some(held_under)) = (
                secrets.get(&leaf.leaf.id),
                self.held_under.get(&leaf.leaf.id),
            ) else {
                return false;
            };
            let key = signer
                .public_key_from_secret(&SecretSource::Encrypted(secret.clone()))
                .await;
            if key.ok().as_ref() != Some(held_under) {
                return false;
            }
        }
        true
    }
}

/// The verifying key is the owner's key plus the operators' share. Applying
/// `tweak` (the old owner key minus the new one) adds it to the share, so the new
/// owner key is the verifying key minus the current share minus the tweak.
fn held_under_after_tweak(
    verifying: &PublicKey,
    operators: &PublicKey,
    tweak: &PublicKey,
) -> Option<PublicKey> {
    let secp = Secp256k1::verification_only();
    verifying
        .combine(&operators.negate(&secp))
        .and_then(|owner| owner.combine(&tweak.negate(&secp)))
        .ok()
}

/// Claims a transfer made to the SSP and takes its leaves into the pool.
///
/// Every claim goes through here, because claiming is what moves a leaf onto the
/// key derived from its node id: a leaf the SSP had fronted from a deposit tree
/// stops signing under the deposit leaf id the moment it comes back. Recording
/// that before the leaves reach the pool is what keeps the two in step, since a
/// spend that read the stale id would sign with a key the operators reject and
/// the leaf would be stuck.
pub async fn claim_into_pool(
    transfer_service: &TransferService,
    signing_keys: &dyn LeafSigningKeys,
    tree_store: &dyn TreeStore,
    transfer: &Transfer,
) -> Result<Vec<TreeNode>, BoxError> {
    // One attempt: retrying a failed claim is left to the caller.
    let claimed = transfer_service
        .claim_transfer(
            transfer,
            Some(ClaimTransferConfig {
                max_retries: 1,
                ..ClaimTransferConfig::default()
            }),
        )
        .await?;
    let node_ids: Vec<String> = claimed.iter().map(|node| node.id.to_string()).collect();
    signing_keys.mark_signing_under_node_id(&node_ids).await?;
    tree_store.add_leaves(&claimed).await?;
    Ok(claimed)
}

/// Returns the leaves a reservation holds to the pool, given their ids.
///
/// The store returns a leaf it is told to keep even when that reservation is
/// gone, so this must not run twice for one reservation: its leaves may have been
/// reserved again in between. Callers forget a reservation before releasing it,
/// leaving its leaves reserved if the release does not happen.
pub async fn release_reserved_leaves(
    tree_store: &dyn TreeStore,
    reservation_id: &LeavesReservationId,
    leaf_ids: &[TreeNodeId],
) -> Result<(), TreeServiceError> {
    let leaves: Vec<TreeNode> = tree_store
        .get_leaves()
        .await?
        .reserved_for_payment
        .into_iter()
        .filter(|leaf| leaf_ids.contains(&leaf.id))
        .collect();
    tree_store.cancel_reservation(reservation_id, &leaves).await
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    use super::held_under_after_tweak;

    #[test]
    fn a_tweaked_leaf_is_held_under_the_new_key() {
        let secp = Secp256k1::new();
        let old = SecretKey::from_slice(&[1; 32]).unwrap();
        let operators = SecretKey::from_slice(&[2; 32]).unwrap();
        let new = SecretKey::from_slice(&[3; 32]).unwrap();
        let verifying = old
            .public_key(&secp)
            .combine(&operators.public_key(&secp))
            .unwrap();
        let tweak = old
            .public_key(&secp)
            .combine(&new.public_key(&secp).negate(&secp))
            .unwrap();

        assert_eq!(
            held_under_after_tweak(&verifying, &operators.public_key(&secp), &tweak),
            Some(new.public_key(&secp))
        );
    }
}
