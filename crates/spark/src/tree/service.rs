use futures::future::join_all;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;
use tracing::{debug, error, info, trace, warn};

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
        select_helper,
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
}

#[macros::async_trait]
impl TreeService for SynchronousTreeService {
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
        new_leaves: Option<&[LeafPedigree]>,
    ) -> Result<(), TreeServiceError> {
        self.state.finalize_reservation(&id, new_leaves).await
    }

    async fn insert_leaves(
        &self,
        leaves: Vec<LeafPedigree>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        // The caller supplies each leaf's ancestors, and a renewal rebuilds the chain
        // of anything it reparents from those ancestors plus the new split node it
        // returns, so the pedigrees come back complete with nothing fetched here.
        let pedigrees = self.check_renew_nodes(leaves).await?;
        self.state.add_leaves(&pedigrees).await?;
        Ok(pedigrees.into_iter().map(|p| p.leaf).collect())
    }

    async fn fetch_pedigrees_from_operators(&self, leaves: &[TreeNode]) -> Vec<LeafPedigree> {
        if leaves.is_empty() {
            return Vec::new();
        }
        let leaf_ids: Vec<TreeNodeId> = leaves.iter().map(|l| l.id.clone()).collect();
        match self.fetch_nodes(&leaf_ids, true).await {
            Ok(nodes) => {
                let mut node_map: HashMap<TreeNodeId, TreeNode> =
                    nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
                // The passed leaves are authoritative over the query's copy.
                for leaf in leaves {
                    node_map.insert(leaf.id.clone(), leaf.clone());
                }
                assemble_exit_chains(&node_map, &leaf_ids)
            }
            Err(e) => {
                // Not fatal: keep the leaves, drop the ancestors. A later refresh
                // fills the chains; until then these leaves are not offline-exitable.
                warn!(
                    "Failed to fetch ancestors from operators, storing leaves without them: {e:?}"
                );
                leaves
                    .iter()
                    .map(|leaf| LeafPedigree {
                        leaf: leaf.clone(),
                        ancestors: Vec::new(),
                    })
                    .collect()
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

        // Prepare queries for coordinator and all operators and run them in parallel
        let coordinator_client = self.operator_pool.get_coordinator().client.clone();
        let operators: Vec<_> = self
            .operator_pool
            .get_non_coordinator_operators()
            .map(|op| (op.id, op.client.clone()))
            .collect();

        // Ask the coordinator for each leaf's ancestors in the same round-trip
        // (`include_parents`), so the pedigree builds below complete their chains
        // from this response instead of re-fetching. The operator queries only feed
        // the leaf-level comparison, so they stay leaves-only.
        let coord_fut =
            self.query_nodes(&coordinator_client, true, None, available_leaf_statuses());
        let op_futs = operators.iter().map(|(id, client)| async move {
            (
                *id,
                self.query_nodes(client, false, None, available_leaf_statuses())
                    .await,
            )
        });

        let (coordinator_leaves_res, operator_results) = tokio::join!(coord_fut, join_all(op_futs));
        // Split the coordinator response: the Available nodes are the leaves (what
        // the query matched); the rest are the ancestors it included, kept as a
        // seed. Every downstream consumer uses the Available leaves exactly as
        // before, so this keeps behavior identical when `include_parents` returns
        // nothing extra.
        let (coordinator_leaves, ancestor_seed): (Vec<TreeNode>, Vec<TreeNode>) =
            coordinator_leaves_res?
                .into_iter()
                .partition(|n| n.status == TreeNodeStatus::Available);
        debug!(
            leaves = coordinator_leaves.len(),
            seeded_ancestors = ancestor_seed.len(),
            "refresh_leaves: fetched leaves and their ancestors in one query"
        );

        // Propagate any operator query error to preserve original behavior and
        // collect successful operator leaves for later comparison
        let mut operator_leaves_vec: Vec<Vec<TreeNode>> = Vec::new();
        for (id, res) in operator_results {
            match res {
                Ok(leaves) => operator_leaves_vec.push(leaves),
                Err(e) => {
                    error!("Failed to query operator {id}: {e:?}");
                    return Err(e);
                }
            }
        }

        let mut missing_operator_leaves_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();
        let mut ignored_leaves_map: HashMap<TreeNodeId, TreeNode> = HashMap::new();

        // For each operator's leaves, compare against coordinator in the same way as before
        for (operator_id, operator_leaves) in operators.into_iter().zip(operator_leaves_vec) {
            for leaf in &coordinator_leaves {
                match operator_leaves.iter().find(|l| l.id == leaf.id) {
                    Some(operator_leaf) => {
                        // TODO: move this logic to TreeNode method
                        if operator_leaf.status != leaf.status
                            || operator_leaf.signing_keyshare.public_key
                                != leaf.signing_keyshare.public_key
                            || operator_leaf.node_tx != leaf.node_tx
                            || operator_leaf.refund_tx != leaf.refund_tx
                        {
                            warn!(
                                "Ignoring leaf due to mismatch between coordinator and operator {}. Coordinator: {:?}, Operator: {:?}",
                                operator_id.0, leaf, operator_leaf
                            );
                            missing_operator_leaves_map.insert(leaf.id.clone(), leaf.clone());
                        }
                    }
                    None => {
                        warn!(
                            "Ignoring leaf due to missing from operator {}: {:?}",
                            operator_id.0, leaf.id
                        );
                        missing_operator_leaves_map.insert(leaf.id.clone(), leaf.clone());
                    }
                }
            }
        }

        // Leaves not Available are ignored outright; the rest need an ownership
        // check (our signing share + the operators' share must equal the
        // verifying key).
        let available_leaves: Vec<&TreeNode> = coordinator_leaves
            .iter()
            .filter(|leaf| {
                if leaf.status == TreeNodeStatus::Available {
                    true
                } else {
                    info!("Ignoring leaf {} due to status: {:?}", leaf.id, leaf.status);
                    ignored_leaves_map.insert(leaf.id.clone(), (*leaf).clone());
                    false
                }
            })
            .collect();

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
        let unverified_leaves: Vec<&TreeNode> = available_leaves
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

        let new_leaves = coordinator_leaves
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
        // The coordinator's `include_parents` response already carries every reported
        // leaf's full chain (as the seed), which we trust as complete, so assemble the
        // pedigrees in memory: no store read, no extra fetch. Missing-from-operator
        // leaves are still coordinator-reported, so they draw from the same seed and
        // stay offline-exitable.
        let mut node_map: HashMap<TreeNodeId, TreeNode> = ancestor_seed
            .into_iter()
            .map(|n| (n.id.clone(), n))
            .collect();
        for leaf in new_leaves.iter().chain(missing_operator_leaves.iter()) {
            node_map.insert(leaf.id.clone(), leaf.clone());
        }
        let missing_operator_ids: Vec<TreeNodeId> = missing_operator_leaves
            .iter()
            .map(|l| l.id.clone())
            .collect();
        let new_leaf_ids: Vec<TreeNodeId> = new_leaves.iter().map(|l| l.id.clone()).collect();
        let missing_operator_pedigrees = assemble_exit_chains(&node_map, &missing_operator_ids);
        let refreshed_pedigrees = assemble_exit_chains(&node_map, &new_leaf_ids);
        let pedigrees = self.check_renew_nodes(refreshed_pedigrees).await?;
        self.state
            .set_leaves(&pedigrees, &missing_operator_pedigrees, refresh_started_at)
            .await?;
        Ok(())
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        self.state.get_available_balance().await
    }
}

impl SynchronousTreeService {
    pub fn new(
        identity_pubkey: PublicKey,
        network: Network,
        operator_pool: Arc<OperatorPool>,
        state: Arc<dyn TreeStore>,
        timelock_manager: Arc<TimelockManager>,
        spark_signer: Arc<dyn SparkSigner>,
        swap_service: Arc<Swap>,
    ) -> Self {
        SynchronousTreeService {
            identity_pubkey,
            network,
            operator_pool,
            state,
            timelock_manager,
            spark_signer,
            swap_service,
        }
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

        let new_leaves = match swap_result {
            Ok(leaves) => leaves,
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

        // The swap outputs already carry the ancestors fetched during the swap, so
        // they go into the pool complete and stay offline-exitable.
        let reserved_ids: HashSet<_> = reserved_leaves.iter().map(|l| l.id.clone()).collect();
        let (reserved_pedigrees, change_pedigrees): (Vec<LeafPedigree>, Vec<LeafPedigree>) =
            new_leaves
                .into_iter()
                .partition(|p| reserved_ids.contains(&p.leaf.id));

        // Update the existing reservation with the selected leaves.
        // Change leaves are added to the pool atomically, preventing
        // race conditions where another request could grab them.
        let update_result = self
            .state
            .update_reservation(&reservation.id, &reserved_pedigrees, &change_pedigrees)
            .await;

        match update_result {
            Ok(final_reservation) => {
                trace!(
                    "Selected leaves got reservation after swap: {:?} ({})",
                    final_reservation.id,
                    final_reservation.sum()
                );
                Ok(final_reservation)
            }
            Err(e) => {
                // Update failed - finalize the reservation to release the permit.
                // We use finalize (not cancel) because the OLD leaves were
                // consumed by the swap and no longer exist.
                // Preserve the new swap output in the pool with the ancestors it came with.
                error!("Failed to update reservation after swap: {e:?}, finalizing");
                let pedigrees: Vec<LeafPedigree> = reserved_pedigrees
                    .into_iter()
                    .chain(change_pedigrees)
                    .collect();
                if let Err(finalize_err) = self
                    .state
                    .finalize_reservation(&reservation.id, Some(&pedigrees))
                    .await
                {
                    error!("Failed to finalize reservation after update error: {finalize_err:?}");
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

        warn!(
            "leaf_lifecycle cancel_verify_dropped_all: reservation={} reason=query_failed error={:?}",
            reservation.id, last_err
        );
        Vec::new()
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
    ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
        self.timelock_manager
            .check_renew_nodes(pedigrees)
            .await
            .map_err(|e| TreeServiceError::Generic(format!("Failed to check time lock: {e:?}")))
    }

    async fn renew_leaves_timelocks(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        let mut needs_renewal = false;
        for leaf in &leaves {
            if leaf.needs_refund_tx_renewed()? {
                needs_renewal = true;
                break;
            }
        }
        if !needs_renewal {
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
        let mut needs_renewal = false;
        for leaf in &reservation.leaves {
            if leaf.needs_refund_tx_renewed()? {
                needs_renewal = true;
                break;
            }
        }
        if !needs_renewal {
            return Ok(reservation);
        }

        let id = reservation.id.clone();
        let cancel_input = reservation.clone();

        // The reserved leaves' chains are already in local storage (gc keeps a
        // reservation's ancestors), so read them there rather than fetching.
        let leaf_ids: Vec<TreeNodeId> = reservation.leaves.iter().map(|l| l.id.clone()).collect();
        let pedigrees = self.state.get_exit_chains(&leaf_ids).await?;

        let renewed = match self.check_renew_nodes(pedigrees).await {
            Ok(renewed) => renewed,
            Err(e) => {
                if let Err(err) = self.cancel_reservation(cancel_input).await {
                    error!("Failed to cancel reservation: {err:?}");
                }
                return Err(e);
            }
        };

        // Persist so a later cancel or finalize returns the (possibly renewed) leaves
        // with a complete chain from local state.
        self.state.add_leaves(&renewed).await?;
        let new_leaves = renewed.into_iter().map(|p| p.leaf).collect();
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
    ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
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

        // The swap outputs are fresh, so resolve their ancestors from the operators
        // (best-effort), then renew any expiring timelock (renewal rebuilds a
        // reparented leaf's chain from these). Not added to the store yet: the caller
        // adds them atomically with the reservation update.
        let pedigrees = self.fetch_pedigrees_from_operators(&claimed_nodes).await;
        self.check_renew_nodes(pedigrees).await
    }
}

/// A leaf's ancestors, child first, walking `parent_node_id` through `nodes` and
/// stopping at the root, a gap, or a cycle in the semi-trusted parent ids.
fn walk_ancestors(leaf: &TreeNode, nodes: &HashMap<TreeNodeId, TreeNode>) -> Vec<TreeNode> {
    let mut ancestors = Vec::new();
    let mut visited: HashSet<TreeNodeId> = HashSet::new();
    let mut current = leaf.parent_node_id.clone();
    while let Some(id) = current {
        if !visited.insert(id.clone()) {
            break;
        }
        let Some(node) = nodes.get(&id) else { break };
        current = node.parent_node_id.clone();
        ancestors.push(node.clone());
    }
    ancestors
}

/// Shapes a batch of exit chains from a node lookup. A store loads its leaves and
/// their ancestors into `nodes` with a single query, then calls this to pair each
/// requested leaf with its chain. A leaf id absent from `nodes` is skipped; a chain
/// that hits a gap comes back partial (the exit can still use as much as is present,
/// and completeness is checkable from the root having no parent).
pub fn assemble_exit_chains(
    nodes: &HashMap<TreeNodeId, TreeNode>,
    leaf_ids: &[TreeNodeId],
) -> Vec<LeafPedigree> {
    leaf_ids
        .iter()
        .filter_map(|id| {
            let leaf = nodes.get(id)?.clone();
            let ancestors = walk_ancestors(&leaf, nodes);
            Some(LeafPedigree { leaf, ancestors })
        })
        .collect()
}

fn available_leaf_statuses() -> Vec<i32> {
    vec![ProtoTreeNodeStatus::Available as i32]
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
    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use macros::test_all;
    use uuid::Uuid;

    use super::*;
    use crate::tree::{
        SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus,
        select_helper::{find_exact_multiple_match, find_exact_single_match},
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

    #[test_all]
    fn refresh_query_requests_only_available_leaves() {
        let owner = PublicKey::from_slice(&[2; 33]).unwrap();
        let req = query_nodes_request(
            &owner,
            None,
            false,
            Network::Mainnet,
            &PagingFilter::default(),
            available_leaf_statuses(),
        );
        assert_eq!(req.statuses, vec![ProtoTreeNodeStatus::Available as i32]);
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
