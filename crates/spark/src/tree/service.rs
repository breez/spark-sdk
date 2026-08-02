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

    async fn store_exit_chains(&self, pedigrees: &[LeafPedigree]) -> Result<(), TreeServiceError> {
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
        self.state.finalize_reservation(&id, new_leaves).await
    }

    async fn insert_leaves(
        &self,
        leaves: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, TreeServiceError> {
        // The leaves go in without a chain; a renewal resolves the one parent it
        // needs and returns the reparented leaf with the chain it rebuilt.
        let pedigrees = self.check_renew_nodes(bare_pedigrees(leaves)).await?;
        let leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
        self.state.add_leaves(&leaves).await?;
        self.store_renewed_chains(&pedigrees).await;
        Ok(leaves)
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

        // Prepare queries for coordinator and all operators and run them in parallel
        let coordinator_client = self.operator_pool.get_coordinator().client.clone();
        let operators: Vec<_> = self
            .operator_pool
            .get_non_coordinator_operators()
            .map(|op| (op.id, op.client.clone()))
            .collect();

        // Leaves only. Asking for each leaf's ancestors here would re-download every
        // chain on every refresh, including the ones already stored.
        let coord_fut =
            self.query_nodes(&coordinator_client, false, None, available_leaf_statuses());
        let op_futs = operators.iter().map(|(id, client)| async move {
            (
                *id,
                self.query_nodes(client, false, None, available_leaf_statuses())
                    .await,
            )
        });

        let (coordinator_leaves_res, operator_results) = tokio::join!(coord_fut, join_all(op_futs));
        let coordinator_leaves: Vec<TreeNode> = coordinator_leaves_res?
            .into_iter()
            .filter(|n| n.status == TreeNodeStatus::Available)
            .collect();
        debug!(
            leaves = coordinator_leaves.len(),
            "refresh_leaves: fetched leaves"
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
            // Paging over a set the operator is concurrently mutating can return the
            // same leaf on more than one page, so keep the first occurrence.
            let mut operator_leaves_by_id: HashMap<&TreeNodeId, &TreeNode> =
                HashMap::with_capacity(operator_leaves.len());
            for operator_leaf in &operator_leaves {
                operator_leaves_by_id
                    .entry(&operator_leaf.id)
                    .or_insert(operator_leaf);
            }

            for leaf in &coordinator_leaves {
                match operator_leaves_by_id.get(&leaf.id).copied() {
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
        // A refresh writes no chains, so an already-stored one is left alone
        // rather than rewritten every minute. Collecting the chains of anything
        // newly reported is the caller's to schedule.
        let pedigrees = self.check_renew_nodes(bare_pedigrees(new_leaves)).await?;
        let renewed_leaves: Vec<TreeNode> = pedigrees.iter().map(|p| p.leaf.clone()).collect();
        self.state
            .set_leaves(
                &renewed_leaves,
                &missing_operator_leaves,
                refresh_started_at,
            )
            .await?;
        self.store_renewed_chains(&pedigrees).await;
        Ok(())
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        self.state.get_available_balance().await
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
                self.store_renewed_chains(&renewed).await;
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

    /// Stores the chains a renewal rebuilt. Renewal reparents the leaf, so its
    /// stored chain no longer starts at its parent and has to be replaced; every
    /// other chain is written by the exit chain resolver. Call after the leaves
    /// are in the pool: a chain is only kept for a leaf the store holds.
    ///
    /// Best-effort, and deliberately not fallible to the caller: the leaves are
    /// already committed by then, so failing here would report a completed
    /// operation as failed. A chain left unwritten is reported by
    /// `leaves_missing_exit_chains` and re-resolved in the background.
    async fn store_renewed_chains(&self, pedigrees: &[LeafPedigree]) {
        let renewed: Vec<LeafPedigree> = pedigrees
            .iter()
            .filter(|p| !p.ancestors.is_empty())
            .cloned()
            .collect();
        if renewed.is_empty() {
            return;
        }
        if let Err(e) = self.state.store_ancestors(&renewed).await {
            warn!("Failed to store renewed exit chains: {e:?}");
        }
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
        // from local state.
        let new_leaves: Vec<TreeNode> = renewed.iter().map(|p| p.leaf.clone()).collect();
        self.state.add_leaves(&new_leaves).await?;
        self.store_renewed_chains(&renewed).await;
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

        // The swap outputs go on without their chains: resolving them here would
        // spend a round trip on leaves this operation is about to send back out.
        // Renewal fetches the one parent it needs for an expiring timelock.
        self.check_renew_nodes(bare_pedigrees(claimed_nodes)).await
    }
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

/// Whether `ancestors` is a chain that can back `leaf`'s unilateral exit. A leaf
/// that is itself a root needs no ancestors.
///
/// Only the leaf's current parent is looked for. A stored chain always runs from
/// some parent to a root, so containing the parent the leaf has now means it
/// spans the whole path. Timelock renewal reparents a leaf onto a new split node
/// under the same id, and this is what catches the chain it left behind.
pub fn chain_backs_exit(leaf: &TreeNode, ancestors: &[TreeNode]) -> bool {
    let Some(parent_id) = leaf.parent_node_id.as_ref() else {
        return true;
    };
    ancestors.iter().any(|node| &node.id == parent_id)
}

/// Shapes a batch of exit chains from a node lookup. A store loads its leaves and
/// their ancestors into `nodes` with a single query, then calls this to pair each
/// requested leaf with its chain. A leaf id absent from `nodes` is skipped, and a
/// chain that hits a gap comes back with what it reached; [`chain_backs_exit`] is
/// what decides whether that is enough.
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
