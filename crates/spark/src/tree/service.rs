use futures::future::join_all;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;
use platform_utils::tokio::sync::broadcast;
use tracing::{debug, error, info, trace, warn};

use crate::services::csv_timelock;
use crate::tree::{
    LeafPedigree, LeafSelection, Leaves, ReservationPurpose, ReserveResult, SelectLeavesOptions,
    TreeNodeStatus,
};
use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            SparkRpcClient,
            spark::{
                QueryNodesRequest, TreeNodeIds, TreeNodeStatus as ProtoTreeNodeStatus,
                query_nodes_request::Source,
            },
        },
    },
    services::{Swap, TimelockManager},
    signer::SparkSigner,
    tree::{
        LeavesReservation, LeavesReservationId, TargetAmounts, TreeNodeId, TreeService, TreeStore,
        assemble_exit_chains, chain_reaches_root, select_helper,
    },
    utils::paging::{PagingFilter, PagingResult, pager},
};

use super::{TreeNode, error::TreeServiceError};

pub struct SynchronousTreeService {
    identity_pubkey: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool>,
    state: Arc<dyn TreeStore>,
    timelock_manager: Arc<TimelockManager>,
    spark_signer: Arc<dyn SparkSigner>,
    swap_service: Arc<Swap>,
    leaves_added: broadcast::Sender<()>,
    /// Published to when a timelock renewal rebuilds the transactions a leaf is
    /// exited with, replacing the data a unilateral exit is built from.
    exit_state_changed: Option<tokio::sync::watch::Sender<()>>,
    /// Stands in for the renewal the timelock manager performs, so a test can
    /// drive one without an operator to sign it.
    #[cfg(test)]
    renew_stub: Option<RenewStub>,
}

#[cfg(test)]
type RenewStub = Box<dyn Fn(Vec<LeafPedigree>) -> Vec<LeafPedigree> + Send + Sync>;

/// Leaves after a renewal pass, the renewed and the untouched alike. A renewed
/// leaf comes back with rebuilt transactions, usually under the parent it
/// already had; the renewal that injects a split node reparents it onto that.
struct RenewalOutcome {
    pedigrees: Vec<LeafPedigree>,
    /// Whether the pass actually rebuilt a leaf's refund transaction. Not the
    /// same as one being due: the operators can refuse a renewal, and the leaf
    /// is handed back exactly as it came to be tried again next pass, with
    /// nothing for a listener to act on.
    any_renewal_landed: bool,
}

/// Notifications buffered per listener before it starts missing them. Each one
/// says only that the pool grew, so a listener that falls behind loses nothing
/// a single later notification does not tell it just as well.
const LEAVES_ADDED_CAPACITY: usize = 16;

#[macros::async_trait]
impl TreeService for SynchronousTreeService {
    fn subscribe_leaves_added(&self) -> broadcast::Receiver<()> {
        self.leaves_added.subscribe()
    }

    async fn list_leaves(&self) -> Result<Leaves, TreeServiceError> {
        self.state.get_leaves().await
    }

    async fn fetch_nodes(
        &self,
        node_ids: &[TreeNodeId],
        include_parents: bool,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }
        let client = &self.operator_pool.get_coordinator().client;
        let source = Source::NodeIds(TreeNodeIds {
            node_ids: node_ids.iter().map(ToString::to_string).collect(),
        });
        self.query_nodes(client, include_parents, Some(source), vec![])
            .await
    }

    async fn load_exit_chains(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
        // The store resolves every chain in one query; no operator top-up, so this
        // stays usable with the operators offline.
        self.state.get_exit_chains(leaf_ids).await
    }

    async fn store_exit_chains(&self, pedigrees: &[LeafPedigree]) -> Result<(), TreeServiceError> {
        if pedigrees.is_empty() {
            return Ok(());
        }
        self.state.store_ancestors(pedigrees).await?;
        self.signal_exit_state_changed();
        Ok(())
    }

    async fn restore_exit_chains(
        &self,
        pedigrees: &[LeafPedigree],
    ) -> Result<(), TreeServiceError> {
        if pedigrees.is_empty() {
            return Ok(());
        }
        self.state.store_ancestors(pedigrees).await
    }

    async fn leaves_missing_exit_chains(&self) -> Result<Vec<TreeNodeId>, TreeServiceError> {
        self.state.leaves_missing_exit_chains().await
    }

    async fn cancel_reservation(
        &self,
        reservation: LeavesReservation,
    ) -> Result<(), TreeServiceError> {
        let leaves_to_keep = self.verify_leaves_against_coordinator(&reservation).await;
        self.state
            .cancel_reservation(&reservation.id, &leaves_to_keep)
            .await
    }

    async fn finalize_reservation(
        &self,
        id: LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        self.state.finalize_reservation(&id, new_leaves).await?;
        // A swap's change, which the reservation being finalized paid for.
        self.notify_leaves_added(new_leaves.unwrap_or_default());
        Ok(())
    }

    async fn insert_leaves(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        // The leaves go in without a chain; a renewal resolves the one parent it
        // needs and returns the reparented leaf with the chain it rebuilt.
        let RenewalOutcome {
            pedigrees,
            any_renewal_landed,
        } = self.check_renew_nodes(bare_pedigrees(leaves)).await?;
        let leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
        self.state.add_leaves(&leaves).await?;
        self.store_renewed_chains(&pedigrees, any_renewal_landed)
            .await;
        self.notify_leaves_added(&leaves);
        Ok(leaves)
    }

    async fn restore_leaves(&self, leaves: &[LeafPedigree]) -> Result<(), TreeServiceError> {
        let nodes: Vec<TreeNode> = leaves.iter().map(|p| p.leaf.clone()).collect();
        self.state.add_leaves(&nodes).await?;
        // The chains ride in on the backup rather than the leaf lifecycle, so they
        // go through the ancestor-only write, and only once they still span their
        // leaf: an import predating a renewal describes a parent it no longer has.
        let restorable: Vec<LeafPedigree> = leaves
            .iter()
            .filter(|p| !p.ancestors.is_empty() && chain_reaches_root(&p.leaf, &p.ancestors))
            .cloned()
            .collect();
        self.restore_exit_chains(&restorable).await
    }

    async fn fetch_pedigrees_from_operators(&self, leaf_ids: &[TreeNodeId]) -> Vec<LeafPedigree> {
        if leaf_ids.is_empty() {
            return Vec::new();
        }
        match self.fetch_nodes(leaf_ids, true).await {
            Ok(nodes) => {
                let node_map: HashMap<TreeNodeId, TreeNode> =
                    nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
                assemble_exit_chains(&node_map, leaf_ids)
            }
            Err(e) => {
                // Not fatal: these leaves stay un-exitable offline until a later
                // attempt resolves them.
                warn!("Failed to fetch ancestors from operators: {e:?}");
                Vec::new()
            }
        }
    }

    /// Selects leaves from the tree that sum up to exactly the target amounts.
    /// If such a combination of leaves does not exist, it performs a swap to get a set of leaves matching the target amounts.
    /// If no leaves can be selected, returns an error.
    ///
    /// Uses notification-based waiting: if balance is insufficient but pending
    /// balance from in-flight swaps would help, waits for balance changes
    /// instead of failing immediately (unless `options.max_wait_for_pending` is `Duration::ZERO`).
    async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        purpose: ReservationPurpose,
        options: SelectLeavesOptions,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!(
            "Selecting leaves for target amounts: {target_amounts:?}, purpose: {purpose:?}, options: {options:?}"
        );

        let max_wait = options.max_wait_for_pending;

        let mut balance_rx = self.state.subscribe_balance_changes();
        let wait_start = platform_utils::time::Instant::now();
        let mut wait_count = 0u32;

        loop {
            let reserve_result = self
                .state
                .try_reserve_leaves(target_amounts, false, purpose)
                .await?;

            // Handle non-success cases first with early continue/return
            let reservation = match reserve_result {
                ReserveResult::InsufficientFunds => {
                    return Err(TreeServiceError::InsufficientFunds);
                }
                ReserveResult::WaitForPending {
                    needed,
                    available,
                    pending,
                } => {
                    // If configured for no waiting, return InsufficientFunds immediately
                    if max_wait.is_zero() {
                        return Err(TreeServiceError::InsufficientFunds);
                    }
                    info!(
                        "Waiting for pending balance: available={available}, needed={needed}, pending={pending}",
                    );
                    self.wait_for_pending_balance(
                        &mut balance_rx,
                        &wait_start,
                        &mut wait_count,
                        max_wait,
                        available,
                        pending,
                    )
                    .await?;
                    continue;
                }
                ReserveResult::Success(r) => r,
            };

            let reservation = self.renew_reservation_timelocks(reservation).await?;

            // Check if swap is needed
            if self.reservation_matches_target(&reservation, target_amounts) {
                trace!("Selected leaves match requirements, no swap needed");
                return Ok(reservation);
            }

            // Perform swap and update reservation
            return self
                .perform_swap_and_update_reservation(reservation, target_amounts)
                .await;
        }
    }

    async fn reserve_leaves_by_ids(
        &self,
        leaf_ids: &[TreeNodeId],
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        self.state
            .try_reserve_leaves_by_ids(leaf_ids, purpose)
            .await
    }

    async fn select_leaves_for_package(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeafSelection, TreeServiceError> {
        match self.state.try_select_leaves(target_amounts).await? {
            LeafSelection::Exact(leaves) => Ok(LeafSelection::Exact(
                self.renew_leaves_timelocks(leaves).await?,
            )),
            LeafSelection::SwapNeeded(leaves) => Ok(LeafSelection::SwapNeeded(
                self.renew_leaves_timelocks(leaves).await?,
            )),
        }
    }

    async fn refresh_leaves(&self) -> Result<(), TreeServiceError> {
        // Capture the start time before any network calls from the store's clock.
        // This uses the DB server time for database-backed stores to avoid clock skew.
        // Leaves added after this time will be preserved even if not in the refresh data.
        let refresh_started_at = self.state.now().await?;

        // Query every operator the same way. The coordinator gets no special say in
        // which leaves are ours, so it takes part in the same comparison as the rest.
        // Leaves only: asking for each leaf's ancestors here would re-download every
        // chain on every refresh, including the ones already stored.
        let operators: Vec<_> = self
            .operator_pool
            .get_all_operators()
            .map(|op| (op.id, op.client.clone()))
            .collect();
        let coordinator_id = self.operator_pool.get_coordinator().id;
        let operator_count = operators.len();
        let responses = join_all(operators.iter().map(|(id, client)| async move {
            (
                *id,
                self.query_nodes(client, false, None, held_leaf_statuses())
                    .await,
            )
        }))
        .await;

        // One unreachable operator would shrink the comparison below, making the
        // leaves only it still reports look dropped by everyone, so a refresh is all
        // or nothing.
        let mut per_operator = Vec::with_capacity(responses.len());
        for (id, res) in responses {
            match res {
                Ok(nodes) => per_operator.push((id, nodes)),
                Err(e) => {
                    error!("Failed to query operator {id}: {e:?}");
                    return Err(e);
                }
            }
        }

        // Status cannot pick the leaves out of a response any more: now that a
        // mid-exit leaf is asked for, an ancestor of one is just as able to come
        // back `OnChain` as the leaf below it. Go by shape instead, and drop what
        // another node in the same response calls its parent: an ancestor is not a
        // leaf, whatever status it is in.
        let mut reported: HashMap<TreeNodeId, Vec<(usize, TreeNode)>> = HashMap::new();
        for (id, nodes) in per_operator {
            let parent_ids: HashSet<TreeNodeId> = nodes
                .iter()
                .filter_map(|n| n.parent_node_id.clone())
                .collect();
            for node in nodes {
                if parent_ids.contains(&node.id) {
                    continue;
                }
                reported
                    .entry(node.id.clone())
                    .or_default()
                    .push((id, node));
            }
        }
        debug!(
            leaves = reported.len(),
            operators = operator_count,
            "refresh_leaves: fetched leaves from every operator"
        );

        let mut missing_operator_leaves_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();
        let mut ignored_leaves_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();

        // A leaf every operator reported identically is clean. One that some of them
        // dropped, or described differently, is still ours and still counts towards
        // the balance, but it is held back from payments until they agree again. A
        // leaf no operator reports is simply absent from here, and that absence is
        // what the store reads as a deletion: it now takes every operator to drop a
        // leaf, where before the coordinator alone decided.
        let mut union_leaves: Vec<TreeNode> = Vec::with_capacity(reported.len());
        for (leaf_id, copies) in reported {
            // Where the operators disagree no copy is the right one, so keep the
            // coordinator's, the one the rest of the client transacts against. The
            // leaf is held back either way.
            let Some(representative) = copies
                .iter()
                .find(|(id, _)| *id == coordinator_id)
                .or_else(|| copies.first())
                .map(|(_, node)| node.clone())
            else {
                continue;
            };
            // Count distinct operators: a paged query can return the same node
            // twice for one of them, which would otherwise pass for agreement
            // that another operator never gave.
            let reporting: HashSet<usize> = copies.iter().map(|(id, _)| *id).collect();
            let agreed = reporting.len() == operator_count
                && copies
                    .iter()
                    .all(|(_, node)| leaf_copies_agree(node, &representative));
            if !agreed {
                warn!(
                    "Leaf {leaf_id} reported by {} of {operator_count} operators, and not identically by all of them; holding it back from payments",
                    reporting.len()
                );
                missing_operator_leaves_map.insert(leaf_id.clone(), representative.clone());
            }
            union_leaves.push(representative);
        }

        // Every reported leaf needs an ownership check (our signing share plus the
        // operators' share must equal the verifying key). The check is not gated on
        // status: a leaf part-way through an exit is still ours, and dropping it
        // here would strand the chain that exit resumes from.
        let reported_leaves: Vec<&TreeNode> = union_leaves.iter().collect();

        // Deriving our leaf pubkey is a network round-trip on a remote signer, so
        // re-deriving every leaf each refresh would flood the signer and stall
        // payments behind its rate limiter on large wallets. For remote signers we
        // skip leaves already stored with matching keys (see `VerifiedLeafKeys`),
        // re-checking only new or changed leaves. Local signers derive cheaply, so
        // they skip the store read and verify every available leaf. Remaining
        // fetches run concurrently, leaving the signer to bound its own
        // concurrency; order is preserved for the zip below.
        let already_verified = if self.spark_signer.is_remote() {
            self.state.get_verified_leaf_keys().await?
        } else {
            HashMap::new()
        };
        let unverified_leaves: Vec<&TreeNode> = reported_leaves
            .iter()
            .copied()
            .filter(|leaf| {
                !already_verified.get(&leaf.id).is_some_and(|keys| {
                    keys.verifying_public_key == leaf.verifying_public_key
                        && keys.signing_keyshare_public_key == leaf.signing_keyshare.public_key
                })
            })
            .collect();

        let signer = &self.spark_signer;
        let our_pubkeys: Vec<PublicKey> = futures::future::try_join_all(
            unverified_leaves
                .iter()
                .map(|leaf| async move { signer.get_public_key_for_leaf(&leaf.id).await }),
        )
        .await?;

        for (leaf, our_node_pubkey) in unverified_leaves.iter().zip(our_pubkeys) {
            let combined_pubkey = our_node_pubkey
                .combine(&leaf.signing_keyshare.public_key)
                .map_err(|_| {
                    TreeServiceError::Generic("Failed to combine public keys".to_string())
                })?;

            if combined_pubkey != leaf.verifying_public_key {
                warn!(
                    "Leaf {}'s verifying public key does not match the expected value",
                    leaf.id
                );
                ignored_leaves_map.insert(leaf.id.clone(), (*leaf).clone());
            }
        }

        let new_leaves = union_leaves
            .into_iter()
            .filter(|leaf| {
                !missing_operator_leaves_map.contains_key(&leaf.id)
                    && !ignored_leaves_map.contains_key(&leaf.id)
            })
            .collect::<Vec<_>>();
        let missing_operator_leaves = missing_operator_leaves_map
            .values()
            .filter(|leaf_id| !ignored_leaves_map.contains_key(&leaf_id.id))
            .cloned()
            .collect::<Vec<_>>();
        // A refresh writes no chains, so an already-stored one is left alone
        // rather than rewritten every minute. Collecting the chains of anything
        // newly reported is what the notification below sets off.

        // Only a spendable leaf goes through the renewal check. The statuses
        // added for the exit are either mid-exit or already locked by the
        // operators: renewing one is at best refused, and a node that no longer
        // carries a refund fails the check outright, which would take the whole
        // refresh down with it.
        let (renewable, held): (Vec<TreeNode>, Vec<TreeNode>) = new_leaves
            .into_iter()
            .partition(|leaf| leaf.status == TreeNodeStatus::Available);
        let RenewalOutcome {
            mut pedigrees,
            any_renewal_landed,
        } = self.check_renew_nodes(bare_pedigrees(renewable)).await?;
        pedigrees.extend(bare_pedigrees(held));
        let renewed_leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
        self.state
            .set_leaves(
                &renewed_leaves,
                &missing_operator_leaves,
                refresh_started_at,
            )
            .await?;
        self.store_renewed_chains(&pedigrees, any_renewal_landed)
            .await;
        // Which of the reported leaves the store already held is the store's to
        // know, so anything reported at all is announced.
        self.notify_leaves_added(&renewed_leaves);
        Ok(())
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        self.state.get_available_balance().await
    }

    async fn list_leaves_kept_for_exit(&self) -> Result<Vec<TreeNode>, TreeServiceError> {
        self.state.get_deleted_leaves().await
    }

    async fn purge_proven_spent_leaves(&self) -> Result<usize, TreeServiceError> {
        let deleted = self.state.get_deleted_leaves().await?;
        if deleted.is_empty() {
            return Ok(0);
        }
        let ids: Vec<TreeNodeId> = deleted.iter().map(|leaf| leaf.id.clone()).collect();
        let source = Source::NodeIds(TreeNodeIds {
            node_ids: ids.iter().map(ToString::to_string).collect(),
        });

        // Ask every operator, by id. By id because an owner query is
        // status-filtered, and the operators drop Investigation, Lost and
        // Reimbursed from it whatever statuses we ask for, so it cannot tell a
        // spent leaf from one it will not talk about; a lookup by id answers for
        // a node in any status. Every operator because this ends in a deletion,
        // and no single one of them, coordinator included, gets to decide that on
        // its own.
        let operators: Vec<_> = self
            .operator_pool
            .get_all_operators()
            .map(|op| (op.id, op.client.clone()))
            .collect();
        let operator_count = operators.len();
        if operator_count == 0 {
            return Ok(0);
        }
        let responses = join_all(operators.iter().map(|(id, client)| {
            let source = source.clone();
            async move {
                (
                    *id,
                    self.query_nodes(client, false, Some(source), vec![]).await,
                )
            }
        }))
        .await;

        // An operator we could not reach has not vouched for anything, so its
        // silence holds the leaves rather than counting towards a deletion.
        let mut agreeing: HashMap<TreeNodeId, usize> = HashMap::new();
        for (id, res) in responses {
            let Ok(nodes) = res else {
                debug!("purge: operator {id} unreachable, keeping every leaf this round");
                return Ok(0);
            };
            let theirs: HashMap<TreeNodeId, TreeNode> =
                nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
            for leaf in &deleted {
                if theirs
                    .get(&leaf.id)
                    .is_some_and(|theirs| refund_left_us(&self.identity_pubkey, leaf, theirs))
                {
                    *agreeing.entry(leaf.id.clone()).or_default() += 1;
                }
            }
        }

        let proven: Vec<TreeNodeId> = deleted
            .iter()
            .filter(|leaf| agreeing.get(&leaf.id) == Some(&operator_count))
            .map(|leaf| leaf.id.clone())
            .collect();
        if !proven.is_empty() {
            info!(
                "Removing {} of {} kept leaves: every operator reports their refund replaced at a lower timelock",
                proven.len(),
                deleted.len()
            );
            self.state.remove_leaves(&proven).await?;
        }
        Ok(proven.len())
    }
}

/// Whether the operators' copy of a leaf says it has left us: it funds the same
/// node, it is owned by somebody else, and its refund replaced ours at a lower
/// timelock.
///
/// The owner is what makes this directional. A lower timelock on its own does
/// not mean the leaf was handed on: a claim re-signs at `enforce_timelock`,
/// which rounds down to the interval, so a leaf coming back to us can carry a
/// lower one too (an HTLC refund sits at a 70-block offset and rounds down on
/// claim). Requiring both means a leaf that returned to us is never mistaken for
/// one that left, and anything the operators will not talk about proves nothing
/// and stays exactly where it is.
fn refund_left_us(identity: &PublicKey, ours: &TreeNode, theirs: &TreeNode) -> bool {
    // Their copy has to name an owner, and it has to be someone else. An absent
    // owner is not evidence either way, so it keeps the leaf.
    let Some(owner) = theirs.owner_identity_public_key else {
        return false;
    };
    if owner == *identity {
        return false;
    }
    // And it has to be the same node: one that funds a different transaction is
    // not the leaf we asked about, so its timelock says nothing about ours.
    if ours.node_tx != theirs.node_tx {
        return false;
    }
    let (Some(ours), Some(theirs)) = (&ours.refund_tx, &theirs.refund_tx) else {
        return false;
    };
    match (csv_timelock(ours), csv_timelock(theirs)) {
        (Some(ours), Some(theirs)) => theirs < ours,
        _ => false,
    }
}

impl SynchronousTreeService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity_pubkey: PublicKey,
        network: Network,
        operator_pool: Arc<OperatorPool>,
        state: Arc<dyn TreeStore>,
        timelock_manager: Arc<TimelockManager>,
        spark_signer: Arc<dyn SparkSigner>,
        swap_service: Arc<Swap>,
        exit_state_changed: Option<tokio::sync::watch::Sender<()>>,
    ) -> Self {
        SynchronousTreeService {
            identity_pubkey,
            network,
            operator_pool,
            state,
            timelock_manager,
            spark_signer,
            swap_service,
            leaves_added: broadcast::channel(LEAVES_ADDED_CAPACITY).0,
            exit_state_changed,
            #[cfg(test)]
            renew_stub: None,
        }
    }

    /// Publishes an exit state change. Raised by the write that changed it, once
    /// the write has landed: a listener told any earlier would export the very
    /// state the write is replacing.
    fn signal_exit_state_changed(&self) {
        if let Some(sender) = &self.exit_state_changed {
            let _ = sender.send(());
        }
    }

    /// Announces leaves reaching the pool, so their chains get collected. Errors
    /// when nobody is listening, which is the usual case and not a failure.
    fn notify_leaves_added(&self, leaves: &[TreeNode]) {
        if leaves.is_empty() {
            return;
        }
        let _ = self.leaves_added.send(());
    }

    /// Checks if the reservation already matches the target amounts without needing a swap.
    fn reservation_matches_target(
        &self,
        reservation: &LeavesReservation,
        target_amounts: Option<&TargetAmounts>,
    ) -> bool {
        let total_amount_sats = target_amounts.map(|ta| ta.total_sats()).unwrap_or(0);
        (total_amount_sats == 0 || reservation.sum() == total_amount_sats)
            && select_helper::select_leaves_by_target_amounts(&reservation.leaves, target_amounts)
                .is_ok()
    }

    /// Waits for pending balance to become available.
    async fn wait_for_pending_balance(
        &self,
        balance_rx: &mut tokio::sync::watch::Receiver<()>,
        wait_start: &platform_utils::time::Instant,
        wait_count: &mut u32,
        max_wait: Duration,
        available: u64,
        pending: u64,
    ) -> Result<(), TreeServiceError> {
        *wait_count += 1;
        let elapsed = wait_start.elapsed();

        if elapsed > max_wait {
            warn!(
                "Timeout waiting for pending balance after {:?} ({} attempts): available={available}, pending={pending}",
                elapsed, wait_count
            );
            return Err(TreeServiceError::Generic(format!(
                "Timeout waiting for pending balance after {:?}",
                elapsed
            )));
        }

        trace!(
            "Waiting for pending balance (attempt {}, elapsed {:?}): available={available}, pending={pending}",
            wait_count, elapsed
        );

        let wait_timeout = Duration::from_secs(5);
        match tokio::time::timeout(wait_timeout, balance_rx.changed()).await {
            Ok(Ok(())) => {
                trace!("Balance change notification received, retrying");
            }
            Ok(Err(_)) => {
                return Err(TreeServiceError::Generic("Store closed".into()));
            }
            Err(_) => {
                trace!("Wait timeout after {:?}, retrying anyway", wait_timeout);
            }
        }
        Ok(())
    }

    /// Performs a swap and updates the reservation with the new leaves.
    async fn perform_swap_and_update_reservation(
        &self,
        reservation: LeavesReservation,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeavesReservation, TreeServiceError> {
        trace!("Swapping leaves to match target amount");

        let swap_result = self
            .swap_leaves_internal(&reservation.leaves, target_amounts)
            .await;

        let RenewalOutcome {
            pedigrees: new_leaves,
            any_renewal_landed,
        } = match swap_result {
            Ok(checked) => checked,
            Err(e) => {
                let reserved_leaf_ids: Vec<String> = reservation
                    .leaves
                    .iter()
                    .map(|l| l.id.to_string())
                    .collect();
                warn!(
                    "leaf_lifecycle swap_failed_in_select: reservation={} leaf_ids={:?} error={:?}",
                    reservation.id, reserved_leaf_ids, e
                );
                if let Err(cancel_err) = self.cancel_reservation(reservation).await {
                    error!("Failed to cancel reservation after swap error: {cancel_err:?}");
                }
                return Err(e);
            }
        };

        trace!(
            "Swapped leaves to match target amount, got {} new leaves",
            new_leaves.len()
        );

        // Select the exact leaves that match the target amounts
        let new_leaf_nodes: Vec<TreeNode> = new_leaves.iter().map(|p| p.leaf.clone()).collect();
        let target_leaves =
            select_helper::select_leaves_by_target_amounts(&new_leaf_nodes, target_amounts)?;
        let reserved_leaves = [
            target_leaves.amount_leaves,
            target_leaves.fee_leaves.unwrap_or_default(),
        ]
        .concat();

        let reserved_ids: HashSet<_> = reserved_leaves.iter().map(|l| l.id.clone()).collect();
        let (reserved_pedigrees, change_pedigrees): (Vec<LeafPedigree>, Vec<LeafPedigree>) =
            new_leaves
                .into_iter()
                .partition(|p| reserved_ids.contains(&p.leaf.id));

        let reserved_nodes: Vec<TreeNode> =
            reserved_pedigrees.iter().map(|p| p.leaf.clone()).collect();
        let change_nodes: Vec<TreeNode> = change_pedigrees.iter().map(|p| p.leaf.clone()).collect();

        // Update the existing reservation with the selected leaves.
        // Change leaves are added to the pool atomically, preventing
        // race conditions where another request could grab them.
        let update_result = self
            .state
            .update_reservation(&reservation.id, &reserved_nodes, &change_nodes)
            .await;

        match update_result {
            Ok(final_reservation) => {
                trace!(
                    "Selected leaves got reservation after swap: {:?} ({})",
                    final_reservation.id,
                    final_reservation.sum()
                );
                // The swap outputs are in the pool now, so a chain a renewal
                // rebuilt for one of them can be stored against it.
                let renewed: Vec<LeafPedigree> = reserved_pedigrees
                    .into_iter()
                    .chain(change_pedigrees)
                    .collect();
                self.store_renewed_chains(&renewed, any_renewal_landed)
                    .await;
                self.notify_leaves_added(&change_nodes);
                Ok(final_reservation)
            }
            Err(e) => {
                // Update failed - finalize the reservation to release the permit.
                // We use finalize (not cancel) because the OLD leaves were
                // consumed by the swap and no longer exist.
                // Preserve the new swap output in the pool.
                error!("Failed to update reservation after swap: {e:?}, finalizing");
                let leaves: Vec<TreeNode> =
                    reserved_nodes.into_iter().chain(change_nodes).collect();
                if let Err(finalize_err) = self
                    .state
                    .finalize_reservation(&reservation.id, Some(&leaves))
                    .await
                {
                    error!("Failed to finalize reservation after update error: {finalize_err:?}");
                } else {
                    let renewed: Vec<LeafPedigree> = reserved_pedigrees
                        .into_iter()
                        .chain(change_pedigrees)
                        .collect();
                    self.store_renewed_chains(&renewed, any_renewal_landed)
                        .await;
                }
                Err(e)
            }
        }
    }

    async fn verify_leaves_against_coordinator(
        &self,
        reservation: &LeavesReservation,
    ) -> Vec<TreeNode> {
        if reservation.leaves.is_empty() {
            return Vec::new();
        }

        let node_ids: Vec<String> = reservation
            .leaves
            .iter()
            .map(|l| l.id.to_string())
            .collect();
        let coordinator_client = self.operator_pool.get_coordinator().client.clone();

        const MAX_ATTEMPTS: u32 = 3;
        const BASE_DELAY_MS: u64 = 100;
        const MAX_DELAY_MS: u64 = 1000;

        let mut last_err: Option<TreeServiceError> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            let source = Source::NodeIds(TreeNodeIds {
                node_ids: node_ids.clone(),
            });
            match self
                .query_nodes(&coordinator_client, false, Some(source), vec![])
                .await
            {
                Ok(nodes) => {
                    let by_id: HashMap<TreeNodeId, TreeNode> =
                        nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
                    let mut keep: Vec<TreeNode> = Vec::new();
                    let mut dropped: Vec<(TreeNodeId, String)> = Vec::new();
                    for original in &reservation.leaves {
                        match by_id.get(&original.id) {
                            Some(fresh) => {
                                let owned = fresh
                                    .owner_identity_public_key
                                    .map(|owner| owner == self.identity_pubkey)
                                    .unwrap_or(false);
                                if !owned {
                                    dropped.push((original.id.clone(), "not_owned".to_string()));
                                } else if fresh.status != TreeNodeStatus::Available {
                                    dropped.push((
                                        original.id.clone(),
                                        format!("status:{:?}", fresh.status),
                                    ));
                                } else {
                                    keep.push(fresh.clone());
                                }
                            }
                            None => dropped
                                .push((original.id.clone(), "missing_from_response".to_string())),
                        }
                    }
                    if dropped.is_empty() {
                        debug!(
                            "leaf_lifecycle cancel_verified_available: reservation={} kept={}",
                            reservation.id,
                            keep.len()
                        );
                    } else {
                        warn!(
                            "leaf_lifecycle cancel_verify_dropped: reservation={} kept={} dropped={:?}",
                            reservation.id,
                            keep.len(),
                            dropped
                        );
                    }
                    return keep;
                }
                Err(e) => {
                    warn!(
                        "leaf_lifecycle cancel_verify_query_attempt: reservation={} attempt={}/{} error={:?}",
                        reservation.id, attempt, MAX_ATTEMPTS, e
                    );
                    last_err = Some(e);
                    if attempt < MAX_ATTEMPTS {
                        let delay_ms = (BASE_DELAY_MS * 2u64.pow(attempt - 1)).min(MAX_DELAY_MS);
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }

        // The query is what tells us anything about these leaves, so failing it
        // tells us nothing. Hand them all back: the operators being unreachable
        // is the case this whole store exists for, and it is not a reason to
        // stop believing in leaves we hold.
        warn!(
            "leaf_lifecycle cancel_verify_query_failed: reservation={} keeping={} error={:?}",
            reservation.id,
            reservation.leaves.len(),
            last_err
        );
        reservation.leaves.clone()
    }

    async fn query_nodes_inner(
        &self,
        client: &SparkRpcClient,
        include_parents: bool,
        source: Option<Source>,
        statuses: Vec<i32>,
        paging: PagingFilter,
    ) -> Result<PagingResult<TreeNode>, TreeServiceError> {
        let nodes = client
            .query_nodes(query_nodes_request(
                &self.identity_pubkey,
                source,
                include_parents,
                self.network,
                &paging,
                statuses,
            ))
            .await?;
        let items: Vec<TreeNode> = nodes
            .nodes
            .into_values()
            .map(TreeNode::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                TreeServiceError::Generic(format!("Failed to deserialize leaves: {e:?}"))
            })?;

        Ok(PagingResult {
            items,
            next: paging.next_from_offset(nodes.offset),
        })
    }

    async fn query_nodes(
        &self,
        client: &SparkRpcClient,
        include_parents: bool,
        source: Option<Source>,
        statuses: Vec<i32>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let nodes = pager(
            |f| {
                self.query_nodes_inner(client, include_parents, source.clone(), statuses.clone(), f)
            },
            PagingFilter::default(),
        )
        .await?;
        Ok(nodes.items)
    }

    async fn check_renew_nodes(
        &self,
        pedigrees: Vec<LeafPedigree>,
    ) -> Result<RenewalOutcome, TreeServiceError> {
        let before = refund_sequences(&pedigrees);
        #[cfg(test)]
        if let Some(renew) = &self.renew_stub {
            let pedigrees = renew(pedigrees);
            return Ok(RenewalOutcome {
                any_renewal_landed: any_renewal_landed(&before, &pedigrees),
                pedigrees,
            });
        }
        let pedigrees = self
            .timelock_manager
            .check_renew_nodes(pedigrees)
            .await
            .map_err(|e| TreeServiceError::Generic(format!("Failed to check time lock: {e:?}")))?;
        Ok(RenewalOutcome {
            any_renewal_landed: any_renewal_landed(&before, &pedigrees),
            pedigrees,
        })
    }

    /// Stores the chains a renewal rebuilt and publishes the exit state change,
    /// so every renewal path announces itself by calling this rather than by
    /// remembering to. Renewal reparents the leaf, so its stored chain no longer
    /// starts at its parent and has to be replaced; every other chain is written
    /// by the exit chain resolver. Call after the leaves are in the pool: a chain
    /// is only kept for a leaf the store holds.
    ///
    /// Takes what the pass actually renewed rather than what it was owed: a
    /// renewal the operators refuse leaves the leaf as it was and is retried on
    /// the next pass, so announcing one would announce again on every refresh
    /// until it lands.
    ///
    /// Best-effort, and deliberately not fallible to the caller: the leaves are
    /// already committed by then, so failing here would report a completed
    /// operation as failed. A chain left unwritten is reported by
    /// `leaves_missing_exit_chains` and re-resolved in the background.
    async fn store_renewed_chains(&self, pedigrees: &[LeafPedigree], any_renewal_landed: bool) {
        // A pass that renewed nothing hands back the leaves as they arrived, so
        // the chains here are the stored ones read back. Rewriting them, and
        // announcing them, would say an exit state changed that did not.
        if !any_renewal_landed {
            return;
        }
        // A renewal that resolved no ancestors rebuilds a chain of just the new
        // split node, which stops short of a root. Storing it would leave the
        // leaf reading as complete and never fetched again.
        let renewed: Vec<LeafPedigree> = pedigrees
            .iter()
            .filter(|p| !p.ancestors.is_empty() && chain_reaches_root(&p.leaf, &p.ancestors))
            .cloned()
            .collect();
        if !renewed.is_empty()
            && let Err(e) = self.state.store_ancestors(&renewed).await
        {
            warn!("Failed to store renewed exit chains: {e:?}");
        }
        // Announced even with nothing written: the ordinary renewal leaves a leaf
        // where it is and re-signs it there, so the leaf a backup holds is stale
        // though no chain moved.
        self.signal_exit_state_changed();
    }

    async fn renew_leaves_timelocks(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        if !any_needs_renewal(&leaves)? {
            return Ok(leaves);
        }

        let leaf_ids: Vec<TreeNodeId> = leaves.iter().map(|l| l.id.clone()).collect();
        let reservation = self
            .state
            .try_reserve_leaves_by_ids(&leaf_ids, ReservationPurpose::Payment)
            .await?;
        let reservation = self.renew_reservation_timelocks(reservation).await?;
        let renewed = reservation.leaves.clone();
        self.cancel_reservation(reservation).await?;
        Ok(renewed)
    }

    /// Renew timelocks for reserved leaves. On a renewal failure the reservation is
    /// cancelled so its leaves return to the pool.
    async fn renew_reservation_timelocks(
        &self,
        reservation: LeavesReservation,
    ) -> Result<LeavesReservation, TreeServiceError> {
        // Renewal reparents and re-signs a leaf. When no reserved leaf has an
        // expiring refund timelock there is nothing to renew, so skip the storage
        // read and rewrite that would otherwise run on every send.
        if !any_needs_renewal(&reservation.leaves)? {
            return Ok(reservation);
        }

        let id = reservation.id.clone();
        let cancel_input = reservation.clone();

        // The reserved leaves' chains are already in local storage (gc keeps a
        // reservation's ancestors), so read them there rather than fetching.
        let leaf_ids: Vec<TreeNodeId> = reservation.leaves.iter().map(|l| l.id.clone()).collect();
        let pedigrees = self.state.get_exit_chains(&leaf_ids).await?;

        let RenewalOutcome {
            pedigrees,
            any_renewal_landed,
        } = match self.check_renew_nodes(pedigrees).await {
            Ok(checked) => checked,
            Err(e) => {
                if let Err(err) = self.cancel_reservation(cancel_input).await {
                    error!("Failed to cancel reservation: {err:?}");
                }
                return Err(e);
            }
        };

        // Persist so a later cancel or finalize returns the (possibly renewed) leaves
        // from local state.
        let new_leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
        self.state.add_leaves(&new_leaves).await?;
        self.store_renewed_chains(&pedigrees, any_renewal_landed)
            .await;
        // Renewal reparents rather than adds, and rebuilds the chain itself. The
        // rebuild is best-effort though, so a listener is given the chance to
        // notice one that did not make it.
        self.notify_leaves_added(&new_leaves);
        Ok(LeavesReservation::new(new_leaves, id))
    }

    /// Performs a swap operation and returns the new leaves.
    ///
    /// Note: This method does NOT add the new leaves to the store. The caller
    /// is responsible for adding them (e.g., via `update_reservation`
    /// which adds and reserves atomically to avoid race conditions).
    async fn swap_leaves_internal(
        &self,
        leaves: &[TreeNode],
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<RenewalOutcome, TreeServiceError> {
        if leaves.is_empty() {
            return Err(TreeServiceError::Generic("no leaves to swap".to_string()));
        }

        let target_amounts = target_amounts.map(|ta| match ta {
            TargetAmounts::AmountAndFee {
                amount_sats,
                fee_sats,
            } => {
                let mut amounts = vec![*amount_sats];
                if let Some(fee_sats) = fee_sats {
                    amounts.push(*fee_sats);
                }
                amounts
            }
            TargetAmounts::ExactDenominations { denominations } => denominations.clone(),
        });
        let claimed_nodes = self
            .swap_service
            .swap_leaves(leaves, target_amounts)
            .await?;

        // The swap outputs go on without their chains: resolving them here would
        // spend a round trip on leaves this operation is about to send back out.
        // Renewal fetches the one parent it needs for an expiring timelock.
        self.check_renew_nodes(bare_pedigrees(claimed_nodes)).await
    }
}

/// Each leaf's refund timelock, keyed by id. Cheap enough to hold for a whole
/// refresh: a sequence per leaf rather than a second copy of the leaf set.
/// A leaf carrying no refund tx maps to `None` on both sides of the pass, which
/// reads as unchanged.
fn refund_sequences(pedigrees: &[LeafPedigree]) -> HashMap<TreeNodeId, Option<bitcoin::Sequence>> {
    pedigrees
        .iter()
        .map(|p| (p.leaf.id.clone(), p.leaf.refund_sequence().ok()))
        .collect()
}

/// Whether a renewal pass rebuilt any leaf's refund transaction, by comparing the
/// timelocks it handed back with the ones it was given. A renewal always moves
/// that timelock, so a leaf that comes back on the one it arrived with was not
/// renewed, whether or not it was due: the operators refuse one from time to
/// time, and the leaf is retried on the next pass. Deciding from what was due
/// instead would announce a change on every pass until the retry succeeds.
fn any_renewal_landed(
    before: &HashMap<TreeNodeId, Option<bitcoin::Sequence>>,
    after: &[LeafPedigree],
) -> bool {
    after.iter().any(|p| {
        before
            .get(&p.leaf.id)
            .is_none_or(|seq| *seq != p.leaf.refund_sequence().ok())
    })
}

/// Whether any leaf is due for a refund tx renewal, using the same predicate
/// `TimelockManager::check_renew_nodes` partitions on.
fn any_needs_renewal<'a>(
    leaves: impl IntoIterator<Item = &'a TreeNode>,
) -> Result<bool, TreeServiceError> {
    for leaf in leaves {
        if leaf.needs_refund_tx_renewed()? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Leaves as chainless pedigrees, the input shape renewal takes: it fills in the
/// chain only for a leaf it reparents.
fn bare_pedigrees(leaves: Vec<TreeNode>) -> Vec<LeafPedigree> {
    leaves
        .into_iter()
        .map(|leaf| LeafPedigree {
            leaf,
            ancestors: Vec::new(),
        })
        .collect()
}

/// Whether two operators' views of one leaf describe the same leaf. Compares what
/// a client goes on to act upon: the status, the operators' share of the signing
/// key, and the two transactions an exit broadcasts.
fn leaf_copies_agree(a: &TreeNode, b: &TreeNode) -> bool {
    a.status == b.status
        && a.signing_keyshare.public_key == b.signing_keyshare.public_key
        && a.node_tx == b.node_tx
        && a.refund_tx == b.refund_tx
}

/// The statuses a leaf we still hold can be reported in. `Available` is the
/// spendable one. `OnChain` and `Exited` are the two an exit drives a leaf
/// through, and they have to be asked for: a refresh that requested only
/// `Available` lost the leaf from every operator's response at once the moment
/// its node txs confirmed, and the store, reading that absence as a spend,
/// deleted the chain the exit still needed to resume. `ParentExited` is the same
/// problem one level down: exiting one leaf puts every other leaf under that
/// parent into it, so a partial exit would drop the siblings it did not touch.
/// `RenewLocked` is the transient equivalent during a timelock renewal.
///
/// `TransferLocked` is deliberately absent: a leaf of ours in that status is one
/// we are sending, and re-reading it would undo the send.
fn held_leaf_statuses() -> Vec<i32> {
    vec![
        ProtoTreeNodeStatus::Available as i32,
        ProtoTreeNodeStatus::OnChain as i32,
        ProtoTreeNodeStatus::Exited as i32,
        ProtoTreeNodeStatus::ParentExited as i32,
        ProtoTreeNodeStatus::RenewLocked as i32,
    ]
}

fn query_nodes_request(
    identity_pubkey: &PublicKey,
    source: Option<Source>,
    include_parents: bool,
    network: Network,
    paging: &PagingFilter,
    statuses: Vec<i32>,
) -> QueryNodesRequest {
    let source = source.unwrap_or(Source::OwnerIdentityPubkey(
        identity_pubkey.serialize().to_vec(),
    ));
    QueryNodesRequest {
        include_parents,
        limit: paging.limit as i64,
        offset: paging.offset as i64,
        network: network.to_proto_network().into(),
        source: Some(source),
        statuses,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use frost_secp256k1_tr::Identifier;
    use macros::{async_test_all, test_all};
    use uuid::Uuid;

    use super::*;
    use crate::{
        operator::{OperatorConfig, OperatorPoolConfig, rpc::DefaultConnectionManager},
        services::TransferService,
        session_store::{InMemorySessionStore, SessionStore},
        signer::{SparkSignerAdapter, create_test_signer},
        ssp::{RetryConfig, ServiceProvider, ServiceProviderConfig},
        tree::{
            InMemoryTreeStore, SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus,
            select_helper::{find_exact_multiple_match, find_exact_single_match},
            tests::create_test_node_with_parent,
        },
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // Helper function to create test leaves with specific values
    fn create_test_leaves(values: &[u64]) -> Vec<TreeNode> {
        values
            .iter()
            .map(|&value| TreeNode {
                id: TreeNodeId::generate(),
                tree_id: Uuid::now_v7().to_string(),
                value,
                parent_node_id: None,
                node_tx: Transaction {
                    version: Version::non_standard(3),
                    lock_time: LockTime::ZERO,
                    input: vec![],
                    output: vec![],
                },
                refund_tx: None,
                direct_tx: None,
                direct_refund_tx: None,
                direct_from_cpfp_refund_tx: None,
                vout: 0,
                verifying_public_key: PublicKey::from_slice(&[2; 33]).unwrap(),
                owner_identity_public_key: Some(PublicKey::from_slice(&[2; 33]).unwrap()),
                signing_keyshare: SigningKeyshare {
                    owner_identifiers: Vec::new(),
                    threshold: 0,
                    public_key: PublicKey::from_slice(&[2; 33]).unwrap(),
                },
                status: TreeNodeStatus::Available,
            })
            .collect()
    }

    fn empty_leaves() -> Leaves {
        Leaves {
            available: Vec::new(),
            not_available: Vec::new(),
            available_missing_from_operators: Vec::new(),
            reserved_for_payment: Vec::new(),
            reserved_for_swap: Vec::new(),
        }
    }

    #[test_all]
    fn leaf_ids_unions_every_bucket() {
        let leaves = create_test_leaves(&[1_000, 2_000, 3_000]);

        let all_available = Leaves {
            available: leaves.clone(),
            ..empty_leaves()
        };
        let spread = Leaves {
            available: vec![leaves[2].clone()],
            not_available: vec![leaves[2].clone()],
            reserved_for_payment: vec![leaves[0].clone()],
            reserved_for_swap: vec![leaves[1].clone()],
            ..empty_leaves()
        };

        assert_eq!(all_available.leaf_ids().len(), 3);
        assert_eq!(all_available.leaf_ids(), spread.leaf_ids());
    }

    /// A service whose only reachable dependency is `store`, publishing exit
    /// state changes to `exit_state_changed`. Its operator and SSP endpoints are
    /// unroutable, so a test that accidentally leaves local storage fails
    /// instead of reaching a real deployment.
    async fn service_over(
        store: Arc<dyn TreeStore>,
        exit_state_changed: Option<tokio::sync::watch::Sender<()>>,
    ) -> SynchronousTreeService {
        const UNROUTABLE: &str = "http://127.0.0.1:1";

        let spark_signer: Arc<dyn SparkSigner> =
            Arc::new(SparkSignerAdapter::new(Arc::new(create_test_signer())));
        let identity_pubkey = spark_signer.get_identity_public_key().await.unwrap();
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::default());
        let operator_pool_config = OperatorPoolConfig::new(
            0,
            vec![OperatorConfig {
                id: 0,
                identifier: Identifier::try_from(1u16).unwrap(),
                address: UNROUTABLE.to_string(),
                ca_cert: None,
                identity_public_key: identity_pubkey,
                user_agent: None,
            }],
        )
        .unwrap();
        let operator_pool = Arc::new(
            OperatorPool::connect(
                &operator_pool_config,
                Arc::new(DefaultConnectionManager::new()),
                Arc::clone(&session_store),
                Arc::clone(&spark_signer),
                None,
            )
            .await
            .unwrap(),
        );
        let ssp_client = Arc::new(ServiceProvider::new(
            ServiceProviderConfig {
                base_url: UNROUTABLE.to_string(),
                schema_endpoint: None,
                identity_public_key: identity_pubkey,
                user_agent: None,
                retry_config: RetryConfig::default(),
            },
            Arc::clone(&spark_signer),
            session_store,
            None,
        ));
        let transfer_service = Arc::new(TransferService::new(
            Arc::clone(&spark_signer),
            Network::Regtest,
            1,
            Arc::clone(&operator_pool),
            None,
        ));
        let swap_service = Arc::new(Swap::new(
            Network::Regtest,
            Arc::clone(&operator_pool),
            Arc::clone(&spark_signer),
            ssp_client,
            transfer_service,
        ));
        let timelock_manager = Arc::new(TimelockManager::new(
            Arc::clone(&spark_signer),
            Network::Regtest,
            Arc::clone(&operator_pool),
        ));

        SynchronousTreeService::new(
            identity_pubkey,
            Network::Regtest,
            operator_pool,
            store,
            timelock_manager,
            spark_signer,
            swap_service,
            exit_state_changed,
        )
    }

    /// Sets the refund timelock the renewal check reads to decide whether a
    /// leaf's refund is about to expire.
    fn set_refund_sequence(leaf: &mut TreeNode, sequence: u32) {
        leaf.refund_tx = Some(Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                sequence: bitcoin::Sequence::from_consensus(sequence),
                ..Default::default()
            }],
            output: vec![],
        });
    }

    /// A leaf carrying a refund tx with `sequence`.
    fn leaf_with_refund_sequence(id: &str, sequence: u32) -> TreeNode {
        let mut leaf = create_test_node_with_parent(id, Some("root"), TreeNodeStatus::Available);
        set_refund_sequence(&mut leaf, sequence);
        leaf
    }

    #[test_all]
    fn renewal_is_pending_only_for_an_expiring_refund_timelock() {
        let expiring = leaf_with_refund_sequence("expiring", 50);
        let fresh = leaf_with_refund_sequence("fresh", 2_000);

        assert!(any_needs_renewal(std::slice::from_ref(&expiring)).unwrap());
        assert!(!any_needs_renewal(std::slice::from_ref(&fresh)).unwrap());
        assert!(any_needs_renewal(&[fresh, expiring]).unwrap());
    }

    /// The refund timelock a renewal resets a leaf's to, far enough out that a
    /// leaf handed back by one is no longer due.
    const RENEWED_REFUND_SEQUENCE: u32 = 2_000;

    /// What the timelock manager returns for the ordinary renewal: the leaf
    /// where it was, its refund transaction rebuilt.
    fn renew_in_place(pedigrees: Vec<LeafPedigree>) -> Vec<LeafPedigree> {
        pedigrees
            .into_iter()
            .map(|mut pedigree| {
                set_refund_sequence(&mut pedigree.leaf, RENEWED_REFUND_SEQUENCE);
                pedigree
            })
            .collect()
    }

    /// What it returns for a renewal the operators refused: the leaf exactly as
    /// it arrived, to be tried again on the next pass.
    fn renewal_refused(pedigrees: Vec<LeafPedigree>) -> Vec<LeafPedigree> {
        pedigrees
    }

    /// What it returns for the renewal that injects a split node: the same
    /// rebuilt leaf, reparented onto the split node the chain now leads with.
    fn reparent_onto_split(pedigrees: Vec<LeafPedigree>) -> Vec<LeafPedigree> {
        renew_in_place(pedigrees)
            .into_iter()
            .map(|mut pedigree| {
                let split =
                    create_test_node_with_parent("split", Some("root"), TreeNodeStatus::Splitted);
                pedigree.leaf.parent_node_id = Some(split.id.clone());
                pedigree.ancestors.insert(0, split);
                pedigree
            })
            .collect()
    }

    #[async_test_all]
    async fn renewing_a_leaf_signals_the_exit_state_changed() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let mut service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;
        service.renew_stub = Some(Box::new(reparent_onto_split));

        let expiring = leaf_with_refund_sequence("leaf", 50);
        service.insert_leaves(vec![expiring]).await.unwrap();

        assert!(rx.has_changed().unwrap());
        // The chain the renewal rebuilt is the new split node alone, which stops
        // short of a root, so it is not stored: the leaf stays one the exit chain
        // resolver will fetch a whole chain for.
        assert_eq!(stored_chain(&service, "leaf").await, ["leaf"]);
    }

    #[async_test_all]
    async fn a_due_leaf_signals_though_the_renewal_moves_it_nowhere() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let mut service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;
        // The ordinary renewal leaves a leaf where it is, so a signal cannot
        // wait on the tree changing shape around it.
        service.renew_stub = Some(Box::new(renew_in_place));

        let expiring = leaf_with_refund_sequence("leaf", 50);
        service.insert_leaves(vec![expiring]).await.unwrap();

        assert!(rx.has_changed().unwrap());
    }

    #[async_test_all]
    async fn a_due_leaf_the_operators_would_not_renew_stays_quiet() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let mut service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;
        // A refused renewal is not fatal: the leaf is kept as it arrived and
        // retried next pass. It stays due, so deciding the signal on what was due
        // would re-announce on every refresh for an exit state that never moved.
        service.renew_stub = Some(Box::new(renewal_refused));

        let expiring = leaf_with_refund_sequence("leaf", 50);
        service.insert_leaves(vec![expiring]).await.unwrap();

        assert!(!rx.has_changed().unwrap());
    }

    #[async_test_all]
    async fn a_leaf_not_due_stays_quiet_though_the_renewal_moves_it() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let mut service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;
        service.renew_stub = Some(Box::new(reparent_onto_split));

        let fresh = leaf_with_refund_sequence("leaf", 2_000);
        service.insert_leaves(vec![fresh]).await.unwrap();

        // The renewal left the leaf on the timelock it arrived with, so there is
        // nothing an export would hold differently. The rebuilt chain stops short
        // of a root either way, so it is not stored.
        assert_eq!(stored_chain(&service, "leaf").await, ["leaf"]);
        assert!(!rx.has_changed().unwrap());
    }

    #[async_test_all]
    async fn inserting_leaves_without_a_renewal_does_not_signal() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;

        let leaf = leaf_with_refund_sequence("leaf", 2_000);
        let inserted = service.insert_leaves(vec![leaf.clone()]).await.unwrap();

        assert_eq!(inserted, vec![leaf]);
        assert!(!rx.has_changed().unwrap());
    }

    /// Root-to-leaf ids of a stored chain.
    async fn stored_chain(service: &SynchronousTreeService, leaf_id: &str) -> Vec<String> {
        let leaf_id = TreeNodeId::from_str(leaf_id).unwrap();
        let pedigrees = service
            .load_exit_chains(std::slice::from_ref(&leaf_id))
            .await
            .unwrap();
        let pedigree = pedigrees.first().expect("leaf is not stored");
        let mut ids: Vec<String> = pedigree
            .ancestors
            .iter()
            .rev()
            .map(|a| a.id.to_string())
            .collect();
        ids.push(pedigree.leaf.id.to_string());
        ids
    }

    #[async_test_all]
    async fn restore_leaves_keeps_the_given_chain_of_every_leaf() {
        let service = service_over(Arc::new(InMemoryTreeStore::new()), None).await;

        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
        let mid = create_test_node_with_parent("mid", Some("root"), TreeNodeStatus::Splitted);
        let leaf_a = create_test_node_with_parent("leaf-a", Some("mid"), TreeNodeStatus::Available);
        let leaf_b =
            create_test_node_with_parent("leaf-b", Some("root"), TreeNodeStatus::Available);

        service
            .restore_leaves(&[LeafPedigree {
                leaf: leaf_a,
                ancestors: vec![mid, root.clone()],
            }])
            .await
            .unwrap();
        service
            .restore_leaves(&[LeafPedigree {
                leaf: leaf_b,
                ancestors: vec![root],
            }])
            .await
            .unwrap();

        assert_eq!(
            stored_chain(&service, "leaf-a").await,
            ["root", "mid", "leaf-a"]
        );
        assert_eq!(stored_chain(&service, "leaf-b").await, ["root", "leaf-b"]);
    }

    #[async_test_all]
    async fn a_chain_reaching_the_leaf_signals_when_stored() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;

        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        service
            .restore_leaves(&bare_pedigrees(vec![leaf.clone()]))
            .await
            .unwrap();
        // Nothing yet: the leaf went in with no chain, so it is no more exitable
        // offline than it was.
        assert!(!rx.has_changed().unwrap());

        service
            .store_exit_chains(&[LeafPedigree {
                leaf,
                ancestors: vec![root],
            }])
            .await
            .unwrap();

        assert!(rx.has_changed().unwrap());
    }

    #[async_test_all]
    async fn a_chain_replayed_from_a_backup_does_not_signal() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let service = service_over(Arc::new(InMemoryTreeStore::new()), Some(tx)).await;

        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let pedigree = LeafPedigree {
            leaf,
            ancestors: vec![root],
        };
        service
            .restore_leaves(std::slice::from_ref(&pedigree))
            .await
            .unwrap();
        service.restore_exit_chains(&[pedigree]).await.unwrap();

        assert!(!rx.has_changed().unwrap());
    }

    fn pubkey_from(byte: u8) -> PublicKey {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let sk = bitcoin::secp256k1::SecretKey::from_slice(&[byte; 32]).unwrap();
        PublicKey::from_secret_key(&secp, &sk)
    }

    fn other_owner() -> PublicKey {
        pubkey_from(0x22)
    }

    fn us() -> PublicKey {
        pubkey_from(0x11)
    }

    /// A node at `seq`, owned by `owner`.
    fn leaf_owned_at(owner: PublicKey, seq: u32) -> TreeNode {
        let mut node = create_test_leaves(&[1_000]).remove(0);
        node.owner_identity_public_key = Some(owner);
        node.refund_tx = Some(Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                sequence: bitcoin::Sequence::from_consensus(seq),
                ..Default::default()
            }],
            output: vec![],
        });
        node
    }

    /// Deleting takes every operator. Counting agreement per leaf is what
    /// enforces that, so a leaf only one of them vouches for stays put.
    #[test_all]
    fn a_leaf_is_only_proven_when_every_operator_agrees() {
        let ours = leaf_owned_at(us(), 2000);
        let gone = leaf_owned_at(other_owner(), 1900);
        let unchanged = leaf_owned_at(other_owner(), 2000);

        // Three operators, two of which report the replacement.
        let agreeing = [&gone, &gone, &unchanged]
            .iter()
            .filter(|theirs| refund_left_us(&us(), &ours, theirs))
            .count();
        assert_eq!(agreeing, 2);
        assert_ne!(agreeing, 3, "a leaf one operator still vouches for is kept");

        let all = [&gone, &gone, &gone]
            .iter()
            .filter(|theirs| refund_left_us(&us(), &ours, theirs))
            .count();
        assert_eq!(all, 3, "unanimous replacement is what proves the spend");
    }

    /// A leaf has left us only when somebody else owns it AND its refund
    /// replaced ours at a lower timelock. The owner is what makes the test
    /// directional: a claim re-signs through `enforce_timelock`, which rounds
    /// down to the interval, so a leaf coming back to us carries a lower
    /// timelock too and must not be mistaken for one that left.
    #[test_all]
    fn a_leaf_left_only_when_someone_else_owns_it_at_a_lower_timelock() {
        let ours = leaf_owned_at(us(), 2000);

        assert!(refund_left_us(
            &us(),
            &ours,
            &leaf_owned_at(other_owner(), 1900)
        ));

        // The HTLC shape that rounds down on a claim back to us: lower, but ours.
        let ours_htlc = leaf_owned_at(us(), 1970);
        assert!(
            !refund_left_us(&us(), &ours_htlc, &leaf_owned_at(us(), 1900)),
            "a leaf claimed back to us is not a leaf that left"
        );

        // Someone else owns it, but the refund did not move.
        assert!(!refund_left_us(
            &us(),
            &ours,
            &leaf_owned_at(other_owner(), 2000)
        ));
        // A renewal moves the timelock the other way.
        assert!(!refund_left_us(
            &us(),
            &ours,
            &leaf_owned_at(other_owner(), 2100)
        ));

        let mut no_owner = leaf_owned_at(other_owner(), 1900);
        no_owner.owner_identity_public_key = None;
        assert!(!refund_left_us(&us(), &ours, &no_owner));

        let mut no_refund = leaf_owned_at(other_owner(), 1900);
        no_refund.refund_tx = None;
        assert!(!refund_left_us(&us(), &ours, &no_refund));

        // A different node entirely says nothing about ours.
        let mut other_node = leaf_owned_at(other_owner(), 1900);
        other_node.node_tx.lock_time = bitcoin::absolute::LockTime::from_height(7).unwrap();
        assert!(!refund_left_us(&us(), &ours, &other_node));
    }

    /// The refresh asks for the statuses a leaf we hold can be reported in, so a
    /// leaf part-way through an exit keeps coming back instead of vanishing from
    /// every operator at once. `TransferLocked` is deliberately not among them: a
    /// leaf of ours in that status is one we are sending, and re-reading it would
    /// undo the send.
    #[test_all]
    fn refresh_query_requests_held_leaf_statuses() {
        let owner = PublicKey::from_slice(&[2; 33]).unwrap();
        let req = query_nodes_request(
            &owner,
            None,
            false,
            Network::Mainnet,
            &PagingFilter::default(),
            held_leaf_statuses(),
        );
        assert_eq!(
            req.statuses,
            vec![
                ProtoTreeNodeStatus::Available as i32,
                ProtoTreeNodeStatus::OnChain as i32,
                ProtoTreeNodeStatus::Exited as i32,
                ProtoTreeNodeStatus::ParentExited as i32,
                ProtoTreeNodeStatus::RenewLocked as i32,
            ]
        );
        assert!(
            !req.statuses
                .contains(&(ProtoTreeNodeStatus::TransferLocked as i32))
        );
        assert!(matches!(req.source, Some(Source::OwnerIdentityPubkey(_))));
    }

    #[test_all]
    fn node_id_query_is_not_status_filtered() {
        let owner = PublicKey::from_slice(&[2; 33]).unwrap();
        let source = Source::NodeIds(TreeNodeIds {
            node_ids: vec!["n".to_string()],
        });
        let req = query_nodes_request(
            &owner,
            Some(source),
            false,
            Network::Mainnet,
            &PagingFilter::default(),
            vec![],
        );
        assert!(req.statuses.is_empty());
        assert!(matches!(req.source, Some(Source::NodeIds(_))));
    }

    #[test_all]
    fn test_find_exact_single_match() {
        let leaves = create_test_leaves(&[8192, 4096, 2048, 1024]);

        // Should find an exact match
        let result = find_exact_single_match(&leaves, 4096);
        assert!(result.is_some());
        assert_eq!(result.unwrap().value, 4096);

        // Should not find a match
        let result = find_exact_single_match(&leaves, 7000);
        assert!(result.is_none());
    }

    #[test_all]
    fn test_find_exact_multiple_match_simple_case() {
        let leaves = create_test_leaves(&[8192, 4096, 2048, 1024]);

        // Should find 4096 + 1024 = 5120
        let result = find_exact_multiple_match(&leaves, 5120);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 5120);

        // Verify we're using the correct leaves (greedy picks largest first)
        let values: Vec<u64> = selected.iter().map(|leaf| leaf.value).collect();
        assert!(values.contains(&4096));
        assert!(values.contains(&1024));
    }

    #[test_all]
    fn test_find_exact_multiple_match_complex_case() {
        let leaves = create_test_leaves(&[16384, 8192, 4096, 2048, 1024, 512]);

        // Should find a combination adding up to 12288 (8192 + 4096)
        let result = find_exact_multiple_match(&leaves, 12288);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 12288);
    }

    #[test_all]
    fn test_find_exact_multiple_match_edge_cases() {
        // Empty leaves
        let leaves = Vec::<TreeNode>::new();
        assert!(find_exact_multiple_match(&leaves, 1024).is_none());

        // Zero target
        let leaves = create_test_leaves(&[1024, 512]);
        assert_eq!(find_exact_multiple_match(&leaves, 0).unwrap().len(), 0);

        // Impossible combination (greedy picks 8192, remaining=1808, can't make it)
        let leaves = create_test_leaves(&[8192, 4096, 2048]);
        assert!(find_exact_multiple_match(&leaves, 10000).is_none());

        // Target equals single leaf value
        let leaves = create_test_leaves(&[8192, 4096, 2048]);
        let result = find_exact_multiple_match(&leaves, 4096);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value, 4096);
    }

    #[test_all]
    fn test_find_exact_multiple_match_large_values() {
        // Test with larger power-of-two values to ensure our algorithm scales properly
        let leaves = create_test_leaves(&[
            134_217_728, // 2^27
            67_108_864,  // 2^26
            33_554_432,  // 2^25
            16_777_216,  // 2^24
            8_388_608,   // 2^23
        ]);

        // Should find a combination adding up to 100_663_296 (2^26 + 2^25)
        let result = find_exact_multiple_match(&leaves, 100_663_296);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 100_663_296);
    }

    #[test_all]
    fn test_greedy_with_non_power_of_two_success() {
        // [3000, 2048, 1024] targeting 3072
        // Pass 1: picks 3000, remaining=72, can't find → fails
        // Pass 2: filters to [2048, 1024], picks 2048, remaining=1024, picks 1024 → success!
        let leaves = create_test_leaves(&[3000, 2048, 1024]);

        let result = find_exact_multiple_match(&leaves, 3072);
        assert!(result.is_some());

        let selected = result.unwrap();
        let total: u64 = selected.iter().map(|leaf| leaf.value).sum();
        assert_eq!(total, 3072);

        // Verify we selected the power-of-two leaves, not the odd one
        let values: Vec<u64> = selected.iter().map(|leaf| leaf.value).collect();
        assert!(values.contains(&2048));
        assert!(values.contains(&1024));
        assert!(!values.contains(&3000));
    }

    #[test_all]
    fn test_greedy_with_non_power_of_two_failure() {
        // [3000, 2048, 1024] targeting 3080
        // Pass 1: picks 3000, remaining=80, can't find → fails
        // Pass 2: filters to [2048, 1024], picks 2048, remaining=1032, can't find → returns None
        let leaves = create_test_leaves(&[3000, 2048, 1024]);

        let result = find_exact_multiple_match(&leaves, 3080);
        assert!(result.is_none());
    }
}
