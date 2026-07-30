use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use platform_utils::time::SystemTime;

use platform_utils::tokio;
use platform_utils::tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tracing::{info, trace, warn};
use uuid::Uuid;

use crate::tree::{
    LeafPedigree, LeafSelection, Leaves, LeavesReservation, LeavesReservationId,
    ReservationPurpose, ReserveResult, TargetAmounts, TreeNode, TreeNodeId, TreeNodeStatus,
    TreeServiceError, TreeStore, assemble_exit_chains, chain_backs_exit, select_helper,
};

/// Default maximum number of concurrent reservations allowed.
pub const DEFAULT_MAX_CONCURRENT_RESERVATIONS: usize = 30;

/// Default timeout for acquiring a reservation permit.
pub const DEFAULT_RESERVATION_TIMEOUT: Duration = Duration::from_secs(60);

/// A leaf bundled with the timestamp it was added/returned to the pool. A leaf is
/// spendable whenever its status is `Available` and every operator reports it;
/// its ancestor chain matters only at exit time, not for balance or selection.
#[derive(Clone)]
struct StoredLeaf {
    node: TreeNode,
    added_at: SystemTime,
    /// Some operator returned this leaf but at least one other did not. It still
    /// counts towards the wallet balance, but cannot be selected to back a payment
    /// or a swap until every operator reports it again.
    /// Mirrors the `is_missing_from_operators` column of the SQL stores.
    missing_from_operators: bool,
    /// No operator reports this leaf any more. The row is kept rather than
    /// removed, because absence is not proof of a spend and the stored
    /// transactions are the only way to exit the leaf should it still be ours.
    /// The operators cannot even report some of the statuses a leaf of ours can
    /// reach: a `Lost` leaf is filtered out of every owner query, and `Lost` is
    /// exactly the state where they will no longer co-sign a fresh refund. Kept
    /// out of the balance and out of selection, still resolvable for an exit
    /// chain, and removed for good only once a spend has actually been proven.
    deleted: bool,
}

impl StoredLeaf {
    fn from_node(node: &TreeNode, added_at: SystemTime) -> Self {
        Self {
            node: node.clone(),
            added_at,
            missing_from_operators: false,
            deleted: false,
        }
    }

    fn from_node_missing(node: &TreeNode, added_at: SystemTime) -> Self {
        Self {
            node: node.clone(),
            added_at,
            missing_from_operators: true,
            deleted: false,
        }
    }
}

/// Entry in the reservation map, containing leaves, purpose, and the semaphore permit.
/// The permit is automatically released when this entry is dropped.
struct ReservationEntry {
    leaves: Vec<StoredLeaf>,
    purpose: ReservationPurpose,
    /// Semaphore permit held for the duration of this reservation.
    /// Dropped automatically when the reservation is cancelled or finalized.
    _permit: OwnedSemaphorePermit,
    /// Expected change amount that will become available after swap completes.
    /// Used for calculating pending balance.
    pending_change_amount: u64,
}

#[derive(Default)]
struct LeavesState {
    /// Every unreserved leaf, whether or not the operators all reported it.
    leaves: HashMap<TreeNodeId, StoredLeaf>,
    /// Each leaf's ancestor chain, keyed by the leaf's own id rather than by
    /// node id: a node shared by several leaves' chains is stored once per
    /// leaf. A leaf lives only in `leaves` or a reservation; the intermediate
    /// nodes its exit walks through live here.
    ancestors: HashMap<TreeNodeId, Vec<TreeNode>>,
    leaves_reservations: HashMap<LeavesReservationId, ReservationEntry>,
    /// Leaf IDs that have been finalized (spent) with their spent timestamp.
    /// Prevents re-adding during refresh. Cleaned up when leaf is no longer
    /// in refresh data AND was spent before the refresh started.
    spent_leaf_ids: HashMap<TreeNodeId, SystemTime>,
    /// Timestamp of when the most recent swap finalization completed.
    /// Used to detect if a refresh started before a swap finished,
    /// which would cause stale data to be applied.
    last_swap_completed_at: Option<SystemTime>,
}

impl LeavesState {
    /// The pool leaf selection draws from: unreserved, available, and reported
    /// by every operator.
    fn selectable_leaves(&self) -> impl Iterator<Item = &StoredLeaf> {
        self.leaves.values().filter(|stored| {
            stored.node.status == TreeNodeStatus::Available
                && !stored.missing_from_operators
                && !stored.deleted
        })
    }

    /// Total value of the selectable pool. Not the wallet balance:
    /// `TreeStore::get_available_balance` also counts missing-from-operators and
    /// swap-reserved leaves.
    fn available_balance(&self) -> u64 {
        self.selectable_leaves()
            .map(|stored| stored.node.value)
            .sum()
    }

    /// Calculate the total pending balance from in-flight swaps.
    fn pending_balance(&self) -> u64 {
        self.leaves_reservations
            .values()
            .map(|entry| entry.pending_change_amount)
            .sum()
    }
}

/// Commands sent to the store processor.
enum StoreCommand {
    AddLeaves {
        leaves: Vec<TreeNode>,
        response_tx: oneshot::Sender<Result<(), TreeServiceError>>,
    },
    StoreAncestors {
        pedigrees: Vec<LeafPedigree>,
        response_tx: oneshot::Sender<Result<(), TreeServiceError>>,
    },
    GetLeaves {
        response_tx: oneshot::Sender<Result<Leaves, TreeServiceError>>,
    },
    GetDeletedLeaves {
        response_tx: oneshot::Sender<Result<Vec<TreeNode>, TreeServiceError>>,
    },
    RemoveLeaves {
        leaf_ids: Vec<TreeNodeId>,
        response_tx: oneshot::Sender<Result<(), TreeServiceError>>,
    },
    GetExitChains {
        leaf_ids: Vec<TreeNodeId>,
        response_tx: oneshot::Sender<Result<Vec<LeafPedigree>, TreeServiceError>>,
    },
    LeavesMissingExitChains {
        response_tx: oneshot::Sender<Result<Vec<TreeNodeId>, TreeServiceError>>,
    },
    SetLeaves {
        leaves: Vec<TreeNode>,
        missing_operators_leaves: Vec<TreeNode>,
        refresh_started_at: SystemTime,
        response_tx: oneshot::Sender<Result<(), TreeServiceError>>,
    },
    TryReserveLeaves {
        target_amounts: Option<TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
        permit: OwnedSemaphorePermit,
        response_tx: oneshot::Sender<Result<ReserveResult, TreeServiceError>>,
    },
    CancelReservation {
        id: LeavesReservationId,
        leaves_to_keep: Vec<TreeNode>,
        response_tx: oneshot::Sender<Result<(), TreeServiceError>>,
    },
    FinalizeReservation {
        id: LeavesReservationId,
        new_leaves: Option<Vec<TreeNode>>,
        response_tx: oneshot::Sender<Result<(), TreeServiceError>>,
    },
    UpdateReservation {
        reservation_id: LeavesReservationId,
        reserved_leaves: Vec<TreeNode>,
        change_leaves: Vec<TreeNode>,
        response_tx: oneshot::Sender<Result<LeavesReservation, TreeServiceError>>,
    },
    ReserveLeavesByIds {
        leaf_ids: Vec<TreeNodeId>,
        purpose: ReservationPurpose,
        permit: OwnedSemaphorePermit,
        response_tx: oneshot::Sender<Result<LeavesReservation, TreeServiceError>>,
    },
    TrySelectLeaves {
        target_amounts: Option<TargetAmounts>,
        response_tx: oneshot::Sender<Result<LeafSelection, TreeServiceError>>,
    },
}

/// Queue-based in-memory tree store.
///
/// Uses a single processor task to handle all state mutations, eliminating
/// mutex contention. Balance change notifications are broadcast via a watch channel.
/// Concurrent reservations are limited by a configurable semaphore.
///
/// # Lifecycle
///
/// The store spawns a background processor task in `new()`. This task runs until
/// the command channel is closed, which happens automatically when the
/// `InMemoryTreeStore` is dropped (dropping the `command_tx` sender closes the channel).
/// No explicit shutdown is required.
///
/// # Backpressure
///
/// The command channel is bounded to 1024 messages. If the channel fills up
/// (e.g., under extreme load), senders will wait until space is available.
/// This provides natural backpressure to prevent unbounded memory growth.
pub struct InMemoryTreeStore {
    command_tx: mpsc::Sender<StoreCommand>,
    /// Watch channel for balance change notifications.
    balance_changed_rx: watch::Receiver<()>,
    /// Semaphore to limit concurrent reservations.
    reservation_semaphore: Arc<Semaphore>,
    /// Maximum concurrent reservations (stored for trace logging).
    max_concurrent_reservations: usize,
    /// Timeout for acquiring a reservation permit.
    reservation_timeout: Duration,
}

impl InMemoryTreeStore {
    /// Creates a new `InMemoryTreeStore` with default configuration.
    ///
    /// Uses [`DEFAULT_MAX_CONCURRENT_RESERVATIONS`] and [`DEFAULT_RESERVATION_TIMEOUT`].
    pub fn new() -> Self {
        Self::with_config(
            DEFAULT_MAX_CONCURRENT_RESERVATIONS,
            DEFAULT_RESERVATION_TIMEOUT,
        )
    }

    /// Creates a new `InMemoryTreeStore` with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `max_concurrent_reservations` - Maximum number of concurrent reservations allowed.
    ///   Additional reservation attempts will wait up to `reservation_timeout`.
    /// * `reservation_timeout` - How long to wait for a reservation permit before
    ///   returning [`TreeServiceError::ResourceBusy`].
    pub fn with_config(max_concurrent_reservations: usize, reservation_timeout: Duration) -> Self {
        // Bounded channel provides backpressure under extreme load
        let (command_tx, command_rx) = mpsc::channel(1024);
        let (balance_changed_tx, balance_changed_rx) = watch::channel(());
        let reservation_semaphore = Arc::new(Semaphore::new(max_concurrent_reservations));

        // Spawn the processor task - it will exit when command_tx is dropped
        tokio::spawn(Self::run_processor(command_rx, balance_changed_tx));

        Self {
            command_tx,
            balance_changed_rx,
            reservation_semaphore,
            max_concurrent_reservations,
            reservation_timeout,
        }
    }

    /// Main processor loop that handles all commands sequentially.
    async fn run_processor(
        mut command_rx: mpsc::Receiver<StoreCommand>,
        balance_changed_tx: watch::Sender<()>,
    ) {
        let mut state = LeavesState::default();

        while let Some(command) = command_rx.recv().await {
            let balance_before = state.available_balance();
            let pending_before = state.pending_balance();
            // Track if we should always notify (e.g., after swap update)
            let mut force_notify = false;

            match command {
                StoreCommand::AddLeaves {
                    leaves,
                    response_tx,
                } => {
                    let result = Self::process_add_leaves(&mut state, &leaves);
                    let _ = response_tx.send(result);
                }
                StoreCommand::StoreAncestors {
                    pedigrees,
                    response_tx,
                } => {
                    let result = Self::process_store_ancestors(&mut state, &pedigrees);
                    let _ = response_tx.send(result);
                }
                StoreCommand::GetLeaves { response_tx } => {
                    let result = Self::process_get_leaves(&state);
                    let _ = response_tx.send(result);
                }
                StoreCommand::GetDeletedLeaves { response_tx } => {
                    let leaves = state
                        .leaves
                        .values()
                        .filter(|stored| stored.deleted)
                        .map(|stored| stored.node.clone())
                        .collect();
                    let _ = response_tx.send(Ok(leaves));
                }
                StoreCommand::RemoveLeaves {
                    leaf_ids,
                    response_tx,
                } => {
                    // Each leaf owns its chain, so a removed leaf takes its
                    // ancestors with it and nothing else can be orphaned. Only a
                    // leaf still marked goes: the purge read its list, then spent
                    // seconds asking the operators, and a refresh landing in that
                    // window may have brought this one back.
                    for id in &leaf_ids {
                        if state.leaves.get(id).is_some_and(|stored| stored.deleted) {
                            state.leaves.remove(id);
                            state.ancestors.remove(id);
                        }
                    }
                    let _ = response_tx.send(Ok(()));
                }
                StoreCommand::GetExitChains {
                    leaf_ids,
                    response_tx,
                } => {
                    let result = Self::process_get_exit_chains(&state, &leaf_ids);
                    let _ = response_tx.send(result);
                }
                StoreCommand::LeavesMissingExitChains { response_tx } => {
                    let _ = response_tx.send(Ok(Self::process_leaves_missing_exit_chains(&state)));
                }
                StoreCommand::SetLeaves {
                    leaves,
                    missing_operators_leaves,
                    refresh_started_at,
                    response_tx,
                } => {
                    let result = Self::process_set_leaves(
                        &mut state,
                        &leaves,
                        &missing_operators_leaves,
                        refresh_started_at,
                    );
                    let _ = response_tx.send(result);
                }
                StoreCommand::TryReserveLeaves {
                    target_amounts,
                    exact_only,
                    purpose,
                    permit,
                    response_tx,
                } => {
                    let result = Self::process_try_reserve_leaves(
                        &mut state,
                        target_amounts.as_ref(),
                        exact_only,
                        purpose,
                        permit,
                    );
                    let _ = response_tx.send(result);
                }
                StoreCommand::CancelReservation {
                    id,
                    leaves_to_keep,
                    response_tx,
                } => {
                    let result = Self::process_cancel_reservation(&mut state, &id, &leaves_to_keep);
                    let _ = response_tx.send(result);
                }
                StoreCommand::FinalizeReservation {
                    id,
                    new_leaves,
                    response_tx,
                } => {
                    // Permit is automatically released when ReservationEntry is dropped
                    let result =
                        Self::process_finalize_reservation(&mut state, &id, new_leaves.as_deref());
                    let _ = response_tx.send(result);
                }
                StoreCommand::UpdateReservation {
                    reservation_id,
                    reserved_leaves,
                    change_leaves,
                    response_tx,
                } => {
                    let result = Self::process_update_reservation(
                        &mut state,
                        &reservation_id,
                        &reserved_leaves,
                        &change_leaves,
                    );
                    // Always notify after swap update - waiters need to know a swap completed
                    // even if the net balance didn't change (e.g., swap returned exact amount)
                    if result.is_ok() {
                        force_notify = true;
                    }
                    let _ = response_tx.send(result);
                }
                StoreCommand::ReserveLeavesByIds {
                    leaf_ids,
                    purpose,
                    permit,
                    response_tx,
                } => {
                    let result =
                        Self::process_reserve_leaves_by_ids(&mut state, &leaf_ids, purpose, permit);
                    let _ = response_tx.send(result);
                }
                StoreCommand::TrySelectLeaves {
                    target_amounts,
                    response_tx,
                } => {
                    let result = Self::process_try_select_leaves(&state, target_amounts.as_ref());
                    let _ = response_tx.send(result);
                }
            }

            // Notify waiters if available or pending balance changed, or if forced
            let balance_after = state.available_balance();
            let pending_after = state.pending_balance();
            let should_notify =
                balance_after != balance_before || pending_after != pending_before || force_notify;

            if should_notify {
                trace!(
                    "Sending balance notification: available {}→{}, pending {}→{}, force={}",
                    balance_before, balance_after, pending_before, pending_after, force_notify
                );
                // Send notification - subscribers only use this as a trigger, not the value
                let _ = balance_changed_tx.send(());
            }
        }
    }

    fn process_add_leaves(
        state: &mut LeavesState,
        leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let now = SystemTime::now();
        for leaf in leaves {
            let mut updated_in_reservation: Option<LeavesReservationId> = None;
            for (res_id, entry) in &mut state.leaves_reservations {
                if let Some(stored) = entry.leaves.iter_mut().find(|s| s.node.id == leaf.id) {
                    stored.node = leaf.clone();
                    updated_in_reservation = Some(res_id.clone());
                    break;
                }
            }
            if let Some(res_id) = updated_in_reservation {
                info!(
                    "leaf_lifecycle add_leaves: leaf={} value={} updated in reservation={} (skipped re-insert)",
                    leaf.id, leaf.value, res_id
                );
                continue;
            }
            let was_spent = state.spent_leaf_ids.remove(&leaf.id).is_some();
            trace!(
                "leaf_lifecycle add_leaves: leaf={} value={} cleared_spent_marker={}",
                leaf.id, leaf.value, was_spent
            );
            state
                .leaves
                .insert(leaf.id.clone(), StoredLeaf::from_node(leaf, now));
        }
        Ok(())
    }

    fn process_get_leaves(state: &LeavesState) -> Result<Leaves, TreeServiceError> {
        // Separate reserved leaves by purpose
        let mut reserved_for_payment = Vec::new();
        let mut reserved_for_swap = Vec::new();
        for entry in state.leaves_reservations.values() {
            let nodes = entry.leaves.iter().map(|stored| stored.node.clone());
            match entry.purpose {
                ReservationPurpose::Payment => {
                    reserved_for_payment.extend(nodes);
                }
                ReservationPurpose::Swap => {
                    reserved_for_swap.extend(nodes);
                }
            }
        }

        let mut available = Vec::new();
        let mut not_available = Vec::new();
        let mut available_missing_from_operators = Vec::new();
        for stored in state.leaves.values() {
            // A leaf no operator reports any more is kept for its exit chain
            // alone, so it belongs to no bucket: out of the balance, out of
            // selection, still exitable.
            if stored.deleted {
                continue;
            }
            if stored.node.status != TreeNodeStatus::Available {
                not_available.push(stored.node.clone());
            } else if stored.missing_from_operators {
                available_missing_from_operators.push(stored.node.clone());
            } else {
                available.push(stored.node.clone());
            }
        }

        Ok(Leaves {
            available,
            not_available,
            available_missing_from_operators,
            reserved_for_payment,
            reserved_for_swap,
        })
    }

    /// Replaces `leaf_id`'s stored ancestor chain with `nodes`. An empty `nodes`
    /// means the chain is unknown and leaves any already-stored chain alone; a
    /// non-empty chain replaces the previous one wholesale, so a leaf reparented
    /// onto a new branch does not keep nodes it no longer descends from.
    fn upsert_ancestors(
        state: &mut LeavesState,
        leaf_id: &TreeNodeId,
        nodes: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        if nodes.is_empty() {
            return Ok(());
        }
        // Replaced wholesale rather than merged: a chain is one walk, and a node
        // stored under another leaf backs only that leaf's exit.
        state.ancestors.insert(leaf_id.clone(), nodes.to_vec());
        Ok(())
    }

    /// Looks up a leaf by id in the pool or a reservation, without considering
    /// ancestors.
    fn find_stored_leaf(state: &LeavesState, id: &TreeNodeId) -> Option<TreeNode> {
        if let Some(stored) = state.leaves.get(id) {
            return Some(stored.node.clone());
        }
        state.leaves_reservations.values().find_map(|entry| {
            entry
                .leaves
                .iter()
                .find(|s| &s.node.id == id)
                .map(|s| s.node.clone())
        })
    }

    /// Leaf ids whose stored chain cannot back an exit, by the same rule the SQL
    /// stores evaluate in their queries.
    fn process_leaves_missing_exit_chains(state: &LeavesState) -> Vec<TreeNodeId> {
        state
            .leaves
            .values()
            .map(|stored| &stored.node)
            .chain(
                state
                    .leaves_reservations
                    .values()
                    .flat_map(|entry| entry.leaves.iter().map(|stored| &stored.node)),
            )
            .filter(|leaf| {
                let stored = state.ancestors.get(&leaf.id).map_or(&[][..], Vec::as_slice);
                !chain_backs_exit(leaf, stored)
            })
            .map(|leaf| leaf.id.clone())
            .collect()
    }

    fn process_get_exit_chains(
        state: &LeavesState,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
        // Each leaf's chain is assembled from only its own ancestor rows, so a
        // node id shared with another leaf's chain cannot cross-contaminate.
        let mut pedigrees = Vec::with_capacity(leaf_ids.len());
        for leaf_id in leaf_ids {
            let Some(leaf) = Self::find_stored_leaf(state, leaf_id) else {
                continue;
            };
            let mut nodes: HashMap<TreeNodeId, TreeNode> = state
                .ancestors
                .get(leaf_id)
                .into_iter()
                .flatten()
                .map(|n| (n.id.clone(), n.clone()))
                .collect();
            nodes.insert(leaf.id.clone(), leaf);
            pedigrees.extend(assemble_exit_chains(&nodes, std::slice::from_ref(leaf_id)));
        }
        Ok(pedigrees)
    }

    fn process_set_leaves(
        state: &mut LeavesState,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_started_at: SystemTime,
    ) -> Result<(), TreeServiceError> {
        let has_active_swap = state
            .leaves_reservations
            .values()
            .any(|entry| entry.purpose == ReservationPurpose::Swap);
        let swap_completed_during_refresh = state
            .last_swap_completed_at
            .is_some_and(|completed_at| completed_at >= refresh_started_at);
        if has_active_swap || swap_completed_during_refresh {
            info!(
                "leaf_lifecycle set_leaves: SKIP active_swap={has_active_swap} \
                 swap_completed_during_refresh={swap_completed_during_refresh} \
                 refresh_started_at={refresh_started_at:?} \
                 last_swap_completed_at={:?}",
                state.last_swap_completed_at
            );
            return Ok(());
        }
        info!(
            "leaf_lifecycle set_leaves: PROCEED refresh_started_at={refresh_started_at:?} \
             last_swap_completed_at={:?} spent_ids_before_clean={}",
            state.last_swap_completed_at,
            state.spent_leaf_ids.len()
        );

        let cleared_spent: Vec<(TreeNodeId, SystemTime)> = state
            .spent_leaf_ids
            .iter()
            .filter(|(_, spent_at)| **spent_at < refresh_started_at)
            .map(|(id, ts)| (id.clone(), *ts))
            .collect();
        for (id, spent_at) in &cleared_spent {
            info!(
                "leaf_lifecycle set_leaves: clearing spent marker for leaf={} spent_at={:?} refresh_started_at={:?}",
                id, spent_at, refresh_started_at
            );
        }
        state
            .spent_leaf_ids
            .retain(|_, spent_at| *spent_at >= refresh_started_at);

        let old_leaves = std::mem::take(&mut state.leaves);
        let old_leaf_ids: Vec<TreeNodeId> = old_leaves.keys().cloned().collect();

        let now = SystemTime::now();
        for leaf in leaves {
            if !state.spent_leaf_ids.contains_key(&leaf.id) {
                let was_present = old_leaves.contains_key(&leaf.id);
                if !was_present {
                    info!(
                        "leaf_lifecycle set_leaves: re-adding leaf={} value={} (from refresh)",
                        leaf.id, leaf.value
                    );
                }
                state
                    .leaves
                    .insert(leaf.id.clone(), StoredLeaf::from_node(leaf, now));
            } else {
                trace!(
                    "leaf_lifecycle set_leaves: skipped leaf={} (in spent_leaf_ids)",
                    leaf.id
                );
            }
        }
        // Applied after the available leaves so that a leaf the coordinator
        // reports in both lists ends up flagged missing, matching the upsert
        // order of the SQL stores.
        for leaf in missing_operators_leaves {
            if !state.spent_leaf_ids.contains_key(&leaf.id) {
                state
                    .leaves
                    .insert(leaf.id.clone(), StoredLeaf::from_node_missing(leaf, now));
            }
        }

        let mut preserved_count = 0u32;
        let mut deleted_count = 0u32;
        for (id, mut stored) in old_leaves {
            if state.leaves.contains_key(&id) {
                continue;
            }
            // We spent this one ourselves, so its absence is already accounted for
            // and the row can go.
            if state.spent_leaf_ids.contains_key(&id) {
                continue;
            }
            if stored.added_at >= refresh_started_at {
                trace!(
                    "leaf_lifecycle set_leaves: preserved old leaf={} value={} missing_from_operators={} (added after refresh started)",
                    id, stored.node.value, stored.missing_from_operators
                );
                state.leaves.insert(id, stored);
                preserved_count += 1;
                continue;
            }
            // No operator reported it, which is not proof that it was spent. The
            // query cannot even ask for every status a leaf of ours can reach, so
            // an absence may say more about the question than the leaf. Keep the
            // row and its chain, out of the balance and out of selection, until a
            // spend is actually proven.
            if !stored.deleted {
                info!(
                    "leaf_lifecycle set_leaves: marking leaf={} value={} deleted (no operator reports it)",
                    id, stored.node.value
                );
            }
            stored.deleted = true;
            state.leaves.insert(id, stored);
            deleted_count += 1;
        }

        // Update reserved leaves with fresh data, removing them from the unreserved pool
        for entry in state.leaves_reservations.values_mut() {
            for stored in &mut entry.leaves {
                if let Some(fresh) = state.leaves.remove(&stored.node.id) {
                    *stored = fresh;
                }
            }
        }

        // A chain is only ever removed with its leaf: a leaf still in the pool
        // keeps whatever `store_ancestors` wrote for it, and only an id absent
        // from the pool afterward has its chain dropped.
        for id in &old_leaf_ids {
            if !state.leaves.contains_key(id) {
                state.ancestors.remove(id);
            }
        }

        trace!(
            "set_leaves: {} leaves ({} missing from operators, {} kept for their exit chain only), {} preserved from previous state, {} newly unreported",
            state.leaves.len(),
            state
                .leaves
                .values()
                .filter(|stored| stored.missing_from_operators)
                .count(),
            state
                .leaves
                .values()
                .filter(|stored| stored.deleted)
                .count(),
            preserved_count,
            deleted_count
        );

        Ok(())
    }

    /// Writes only the ancestor chain of each pedigree: unlike `process_add_leaves`,
    /// the leaf itself is never touched, so a spent leaf stays spent.
    fn process_store_ancestors(
        state: &mut LeavesState,
        pedigrees: &[LeafPedigree],
    ) -> Result<(), TreeServiceError> {
        for pedigree in pedigrees {
            // The leaf can be spent between its chain being resolved and this write,
            // and a chain is only ever removed with its leaf. Writing one for a leaf
            // that is already gone would leave it behind for good.
            if Self::find_stored_leaf(state, &pedigree.leaf.id).is_none() {
                continue;
            }
            Self::upsert_ancestors(state, &pedigree.leaf.id, &pedigree.ancestors)?;
        }
        Ok(())
    }

    /// Try to reserve - returns `WaitForPending` if should wait.
    /// Automatically tracks pending when reserved > needed.
    /// On success, the permit is stored in the reservation entry.
    /// On non-success, the permit is dropped immediately, releasing the semaphore slot.
    fn process_try_reserve_leaves(
        state: &mut LeavesState,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
        permit: OwnedSemaphorePermit,
    ) -> Result<ReserveResult, TreeServiceError> {
        let target_amount = target_amounts.map_or(0, |ta| ta.total_sats());
        let available = state.available_balance();
        let pending = state.pending_balance();

        // Filter to the selectable pool (available and reported by all operators)
        let leaves: Vec<TreeNode> = state
            .selectable_leaves()
            .map(|stored| stored.node.clone())
            .collect();

        let selected = select_helper::select_leaves_by_target_amounts(&leaves, target_amounts);

        match selected {
            Ok(target_leaves) => {
                // Can satisfy exactly - no pending change needed
                let selected = [
                    target_leaves.amount_leaves,
                    target_leaves.fee_leaves.unwrap_or_default(),
                ]
                .concat();
                let id = Self::reserve_internal(state, &selected, purpose, permit, 0)?;
                Ok(ReserveResult::Success(LeavesReservation::new(selected, id)))
            }
            Err(_) if !exact_only => {
                // Try minimum amount selection (may reserve more than needed)
                if let Ok(Some(selected)) =
                    select_helper::select_leaves_by_minimum_amount(&leaves, target_amount)
                {
                    let reserved_amount: u64 = selected.iter().map(|l| l.value).sum();

                    // Calculate pending change if we reserved more than needed
                    let pending_change_amount =
                        if reserved_amount > target_amount && target_amount > 0 {
                            reserved_amount - target_amount
                        } else {
                            0
                        };

                    let id = Self::reserve_internal(
                        state,
                        &selected,
                        purpose,
                        permit,
                        pending_change_amount,
                    )?;

                    return Ok(ReserveResult::Success(LeavesReservation::new(selected, id)));
                }

                // Can't satisfy now - check if waiting would help
                // Permit is dropped here, releasing the semaphore slot
                if available + pending >= target_amount {
                    Ok(ReserveResult::WaitForPending {
                        needed: target_amount,
                        available,
                        pending,
                    })
                } else {
                    Ok(ReserveResult::InsufficientFunds)
                }
            }
            Err(_) => {
                // Exact only and can't satisfy
                // Permit is dropped here, releasing the semaphore slot
                if available + pending >= target_amount {
                    Ok(ReserveResult::WaitForPending {
                        needed: target_amount,
                        available,
                        pending,
                    })
                } else {
                    Ok(ReserveResult::InsufficientFunds)
                }
            }
        }
    }

    fn process_try_select_leaves(
        state: &LeavesState,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeafSelection, TreeServiceError> {
        let leaves: Vec<TreeNode> = state
            .selectable_leaves()
            .map(|stored| stored.node.clone())
            .collect();

        match select_helper::select_leaves_by_target_amounts(&leaves, target_amounts) {
            Ok(target_leaves) => {
                let selected = [
                    target_leaves.amount_leaves,
                    target_leaves.fee_leaves.unwrap_or_default(),
                ]
                .concat();
                Ok(LeafSelection::Exact(selected))
            }
            Err(_) => {
                let target_amount = target_amounts.map_or(0, |ta| ta.total_sats());
                match select_helper::select_leaves_by_minimum_amount(&leaves, target_amount) {
                    Ok(Some(selected)) => Ok(LeafSelection::SwapNeeded(selected)),
                    _ => Err(TreeServiceError::InsufficientFunds),
                }
            }
        }
    }

    /// Cancel a reservation. The reservation entry and all of its currently-held leaves
    /// are removed; the supplied `leaves_to_keep` are inserted into the available pool
    /// with a fresh `added_at`. To release the original leaves unchanged, callers pass
    /// them as `leaves_to_keep`.
    fn process_cancel_reservation(
        state: &mut LeavesState,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let removed = state.leaves_reservations.remove(id);
        let purpose = removed.as_ref().map(|e| e.purpose);
        let prior_ids: Vec<TreeNodeId> = removed
            .as_ref()
            .map(|e| e.leaves.iter().map(|s| s.node.id.clone()).collect())
            .unwrap_or_default();
        let keep_ids: Vec<&TreeNodeId> = leaves_to_keep.iter().map(|l| &l.id).collect();
        let dropped_ids: Vec<&TreeNodeId> = prior_ids
            .iter()
            .filter(|pid| !keep_ids.contains(pid))
            .collect();
        trace!(
            "leaf_lifecycle cancel: reservation={} purpose={:?} prior_leaves={:?} keeping={:?} dropping={:?}",
            id, purpose, prior_ids, keep_ids, dropped_ids
        );
        // A kept leaf's ancestor chain (owned by its own id) is untouched here;
        // only a dropped leaf's chain goes with it.
        for dropped in &dropped_ids {
            state.ancestors.remove(*dropped);
        }

        let now = SystemTime::now();
        for leaf in leaves_to_keep {
            state
                .leaves
                .insert(leaf.id.clone(), StoredLeaf::from_node(leaf, now));
        }
        Ok(())
    }

    /// Finalize a reservation (leaves are consumed) and optionally add new leaves.
    /// The permit is automatically released when the ReservationEntry is dropped.
    fn process_finalize_reservation(
        state: &mut LeavesState,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let removed = state.leaves_reservations.remove(id);
        if let Some(entry) = &removed {
            let now = SystemTime::now();
            for stored in &entry.leaves {
                trace!(
                    "leaf_lifecycle finalize: marking spent leaf={} reservation={} purpose={:?}",
                    stored.node.id, id, entry.purpose
                );
                state.spent_leaf_ids.insert(stored.node.id.clone(), now);
                state.ancestors.remove(&stored.node.id);
            }

            if entry.purpose == ReservationPurpose::Swap && new_leaves.is_some() {
                state.last_swap_completed_at = Some(now);
            }
        } else {
            warn!("Tried to finalize a non existing reservation");
        }

        if let Some(resulting_leaves) = new_leaves {
            let now = SystemTime::now();
            for leaf in resulting_leaves {
                trace!(
                    "leaf_lifecycle finalize: adding new leaf={} value={} reservation={}",
                    leaf.id, leaf.value, id
                );
                state
                    .leaves
                    .insert(leaf.id.clone(), StoredLeaf::from_node(leaf, now));
            }
        }
        trace!("Finalized leaves reservation: {}", id);
        Ok(())
    }

    /// Updates a reservation after a swap operation.
    /// Replaces the reservation's leaves with `reserved_leaves` and adds
    /// `change_leaves` to the available pool.
    /// The permit is preserved from the original reservation.
    fn process_update_reservation(
        state: &mut LeavesState,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError> {
        // Remove the existing reservation to get the permit
        let old_entry = state
            .leaves_reservations
            .remove(reservation_id)
            .ok_or_else(|| {
                TreeServiceError::Generic(format!("Reservation {} not found", reservation_id))
            })?;
        let purpose = old_entry.purpose;
        let permit = old_entry._permit;
        // The pre-swap reserved leaves are consumed; their chains go with them.
        for stored in &old_entry.leaves {
            state.ancestors.remove(&stored.node.id);
        }

        let now = SystemTime::now();
        for leaf in change_leaves {
            trace!(
                "leaf_lifecycle update_reservation: adding change leaf={} value={} reservation={}",
                leaf.id, leaf.value, reservation_id
            );
            state
                .leaves
                .insert(leaf.id.clone(), StoredLeaf::from_node(leaf, now));
        }

        // Re-insert the reservation with updated leaves but same permit
        // Pending change is cleared since the swap completed
        let reserved: Vec<StoredLeaf> = reserved_leaves
            .iter()
            .map(|leaf| StoredLeaf::from_node(leaf, now))
            .collect();
        state.leaves_reservations.insert(
            reservation_id.clone(),
            ReservationEntry {
                leaves: reserved.clone(),
                purpose,
                _permit: permit,
                pending_change_amount: 0,
            },
        );

        let reserved_nodes: Vec<TreeNode> = reserved.iter().map(|s| s.node.clone()).collect();
        trace!(
            "Updated reservation {}: reserved {} leaves, added {} change leaves to pool",
            reservation_id,
            reserved_nodes.len(),
            change_leaves.len()
        );

        Ok(LeavesReservation::new(
            reserved_nodes,
            reservation_id.clone(),
        ))
    }

    /// Internal helper to reserve leaves (moves them from main pool to reservations).
    /// The permit is stored in the reservation entry and released when the entry is dropped.
    fn reserve_internal(
        state: &mut LeavesState,
        leaves: &[TreeNode],
        purpose: ReservationPurpose,
        permit: OwnedSemaphorePermit,
        pending_change_amount: u64,
    ) -> Result<LeavesReservationId, TreeServiceError> {
        if leaves.is_empty() {
            return Err(TreeServiceError::NonReservableLeaves);
        }
        for leaf in leaves {
            if !state.leaves.contains_key(&leaf.id) {
                return Err(TreeServiceError::NonReservableLeaves);
            }
        }
        let id = Uuid::now_v7().to_string();
        let stored_leaves: Vec<StoredLeaf> = leaves
            .iter()
            .filter_map(|leaf| state.leaves.remove(&leaf.id))
            .collect();
        for stored in &stored_leaves {
            trace!(
                "leaf_lifecycle reserve: leaf={} value={} reservation={} purpose={:?}",
                stored.node.id, stored.node.value, id, purpose
            );
        }
        state.leaves_reservations.insert(
            id.clone(),
            ReservationEntry {
                leaves: stored_leaves,
                purpose,
                _permit: permit,
                pending_change_amount,
            },
        );
        trace!("New leaves reservation {}: {:?}", id, leaves);
        Ok(id)
    }

    fn process_reserve_leaves_by_ids(
        state: &mut LeavesState,
        leaf_ids: &[TreeNodeId],
        purpose: ReservationPurpose,
        permit: OwnedSemaphorePermit,
    ) -> Result<LeavesReservation, TreeServiceError> {
        let unique_ids: HashSet<&TreeNodeId> = leaf_ids.iter().collect();
        if leaf_ids.is_empty() || unique_ids.len() != leaf_ids.len() {
            return Err(TreeServiceError::NonReservableLeaves);
        }
        let mut selected = Vec::with_capacity(leaf_ids.len());
        for id in leaf_ids {
            let stored = state
                .leaves
                .get(id)
                .ok_or(TreeServiceError::NonReservableLeaves)?;
            // A leaf the operators no longer report cannot back a payment or a
            // swap, even though it still counts towards the balance. Nor can one
            // kept only for its exit chain, which counts towards nothing.
            if stored.node.status != TreeNodeStatus::Available
                || stored.missing_from_operators
                || stored.deleted
            {
                return Err(TreeServiceError::NonReservableLeaves);
            }
            selected.push(stored.node.clone());
        }
        let id = Self::reserve_internal(state, &selected, purpose, permit, 0)?;
        Ok(LeavesReservation::new(selected, id))
    }

    /// Sends a command to the processor and returns the response.
    async fn send_command<T>(
        &self,
        f: impl FnOnce(oneshot::Sender<Result<T, TreeServiceError>>) -> StoreCommand,
    ) -> Result<T, TreeServiceError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(f(response_tx))
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }
}

impl Default for InMemoryTreeStore {
    fn default() -> Self {
        Self::new()
    }
}

#[macros::async_trait]
impl TreeStore for InMemoryTreeStore {
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
        let leaves = leaves.to_vec();
        self.send_command(|tx| StoreCommand::AddLeaves {
            leaves,
            response_tx: tx,
        })
        .await
    }

    async fn store_ancestors(&self, pedigrees: &[LeafPedigree]) -> Result<(), TreeServiceError> {
        let pedigrees = pedigrees.to_vec();
        self.send_command(|tx| StoreCommand::StoreAncestors {
            pedigrees,
            response_tx: tx,
        })
        .await
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        self.send_command(|tx| StoreCommand::GetLeaves { response_tx: tx })
            .await
    }

    async fn get_deleted_leaves(&self) -> Result<Vec<TreeNode>, TreeServiceError> {
        self.send_command(|tx| StoreCommand::GetDeletedLeaves { response_tx: tx })
            .await
    }

    async fn remove_leaves(&self, leaf_ids: &[TreeNodeId]) -> Result<(), TreeServiceError> {
        if leaf_ids.is_empty() {
            return Ok(());
        }
        let leaf_ids = leaf_ids.to_vec();
        self.send_command(|tx| StoreCommand::RemoveLeaves {
            leaf_ids,
            response_tx: tx,
        })
        .await
    }

    async fn get_exit_chains(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
        let leaf_ids = leaf_ids.to_vec();
        self.send_command(|tx| StoreCommand::GetExitChains {
            leaf_ids,
            response_tx: tx,
        })
        .await
    }

    async fn leaves_missing_exit_chains(&self) -> Result<Vec<TreeNodeId>, TreeServiceError> {
        self.send_command(|tx| StoreCommand::LeavesMissingExitChains { response_tx: tx })
            .await
    }

    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_started_at: SystemTime,
    ) -> Result<(), TreeServiceError> {
        let leaves = leaves.to_vec();
        let missing_operators_leaves = missing_operators_leaves.to_vec();
        self.send_command(|tx| StoreCommand::SetLeaves {
            leaves,
            missing_operators_leaves,
            refresh_started_at,
            response_tx: tx,
        })
        .await
    }

    async fn cancel_reservation(
        &self,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let id = id.clone();
        let leaves_to_keep = leaves_to_keep.to_vec();
        self.send_command(|tx| StoreCommand::CancelReservation {
            id,
            leaves_to_keep,
            response_tx: tx,
        })
        .await
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let id = id.clone();
        let new_leaves = new_leaves.map(<[TreeNode]>::to_vec);
        self.send_command(|tx| StoreCommand::FinalizeReservation {
            id,
            new_leaves,
            response_tx: tx,
        })
        .await
    }

    async fn try_reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<ReserveResult, TreeServiceError> {
        // Acquire permit with timeout (waits if max_concurrent_reservations are already in use)
        let available_permits = self.reservation_semaphore.available_permits();
        trace!(
            "try_reserve_leaves: waiting for permit (available: {}/{})",
            available_permits, self.max_concurrent_reservations
        );

        let permit = platform_utils::tokio::time::timeout(
            self.reservation_timeout,
            self.reservation_semaphore.clone().acquire_owned(),
        )
        .await
        .map_err(|_| TreeServiceError::ResourceBusy {
            max_concurrent: self.max_concurrent_reservations,
            timeout: self.reservation_timeout,
        })?
        .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        trace!("try_reserve_leaves: acquired permit");

        let target_amounts = target_amounts.cloned();
        self.send_command(|tx| StoreCommand::TryReserveLeaves {
            target_amounts,
            exact_only,
            purpose,
            permit,
            response_tx: tx,
        })
        .await
    }

    async fn try_reserve_leaves_by_ids(
        &self,
        leaf_ids: &[TreeNodeId],
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        let permit = platform_utils::tokio::time::timeout(
            self.reservation_timeout,
            self.reservation_semaphore.clone().acquire_owned(),
        )
        .await
        .map_err(|_| TreeServiceError::ResourceBusy {
            max_concurrent: self.max_concurrent_reservations,
            timeout: self.reservation_timeout,
        })?
        .map_err(|_| TreeServiceError::ProcessorShutdown)?;

        let leaf_ids = leaf_ids.to_vec();
        self.send_command(|tx| StoreCommand::ReserveLeavesByIds {
            leaf_ids,
            purpose,
            permit,
            response_tx: tx,
        })
        .await
    }

    async fn try_select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeafSelection, TreeServiceError> {
        let target_amounts = target_amounts.cloned();
        self.send_command(|tx| StoreCommand::TrySelectLeaves {
            target_amounts,
            response_tx: tx,
        })
        .await
    }

    async fn now(&self) -> Result<SystemTime, TreeServiceError> {
        Ok(SystemTime::now())
    }

    fn subscribe_balance_changes(&self) -> watch::Receiver<()> {
        self.balance_changed_rx.clone()
    }

    async fn update_reservation(
        &self,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError> {
        let reservation_id = reservation_id.clone();
        let reserved_leaves = reserved_leaves.to_vec();
        let change_leaves = change_leaves.to_vec();
        self.send_command(|tx| StoreCommand::UpdateReservation {
            reservation_id,
            reserved_leaves,
            change_leaves,
            response_tx: tx,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests as shared_tests;
    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // ==================== Shared tests ====================

    #[async_test_all]
    async fn test_new() {
        shared_tests::test_new(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_get_verified_leaf_keys() {
        shared_tests::test_get_verified_leaf_keys(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_upsert_and_get_leaf() {
        shared_tests::test_upsert_and_get_leaf(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_get_exit_chains() {
        shared_tests::test_get_exit_chains(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_get_exit_chain_missing_ancestor() {
        shared_tests::test_get_exit_chain_missing_ancestor(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_incomplete_pedigree_still_spendable() {
        shared_tests::test_incomplete_pedigree_still_spendable(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_exit_chain_after_swap_update() {
        shared_tests::test_exit_chain_after_swap_update(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_exit_chain_after_cancel_reparent() {
        shared_tests::test_exit_chain_after_cancel_reparent(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_leaves_missing_exit_chains() {
        shared_tests::test_leaves_missing_exit_chains(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_stored_chain_survives_refresh() {
        shared_tests::test_stored_chain_survives_refresh(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reparented_leaf_needs_its_chain_again() {
        shared_tests::test_reparented_leaf_needs_its_chain_again(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_node_update_in_place() {
        shared_tests::test_node_update_in_place(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_leaf_reparented_by_renewal() {
        shared_tests::test_leaf_reparented_by_renewal(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_ancestor_not_returned_as_leaf() {
        shared_tests::test_ancestor_not_returned_as_leaf(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_absent_leaf_is_kept_for_its_exit_chain() {
        shared_tests::test_absent_leaf_is_kept_for_its_exit_chain(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_remove_leaves_spares_a_revived_leaf() {
        shared_tests::test_remove_leaves_spares_a_revived_leaf(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_kept_leaf_cannot_back_a_payment() {
        shared_tests::test_kept_leaf_cannot_back_a_payment(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_deleted_leaves_are_listed_and_removable() {
        shared_tests::test_deleted_leaves_are_listed_and_removable(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_absent_leaf_keeps_shared_ancestor() {
        shared_tests::test_absent_leaf_keeps_shared_ancestor(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_empty_pedigree_keeps_stored_chain() {
        shared_tests::test_empty_pedigree_keeps_stored_chain(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_stored_chain_replaces_previous() {
        shared_tests::test_stored_chain_replaces_previous(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_store_ancestors_backfills_chain() {
        shared_tests::test_store_ancestors_backfills_chain(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_store_ancestors_does_not_revive_spent_leaf() {
        shared_tests::test_store_ancestors_does_not_revive_spent_leaf(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_store_ancestors_for_absent_leaf() {
        shared_tests::test_store_ancestors_for_absent_leaf(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_add_leaves() {
        shared_tests::test_add_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_add_leaves_duplicate_ids() {
        shared_tests::test_add_leaves_duplicate_ids(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_add_leaves_clears_missing_from_operators() {
        shared_tests::test_add_leaves_clears_missing_from_operators(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_missing_from_operators_leaves_are_not_selectable() {
        shared_tests::test_missing_from_operators_leaves_are_not_selectable(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_missing_from_operators_leaf_not_available() {
        shared_tests::test_missing_from_operators_leaf_not_available(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_set_leaves() {
        shared_tests::test_set_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_across_chunk_boundary() {
        shared_tests::test_set_leaves_across_chunk_boundary(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_with_reservations() {
        shared_tests::test_set_leaves_with_reservations(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
        shared_tests::test_set_leaves_preserves_reservations_for_in_flight_swaps(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_reserve_leaves() {
        shared_tests::test_reserve_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reserve_leaves_by_ids() {
        shared_tests::test_reserve_leaves_by_ids(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reserve_leaves_by_ids_preserves_order() {
        shared_tests::test_reserve_leaves_by_ids_preserves_order(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reserve_leaves_by_ids_not_available() {
        shared_tests::test_reserve_leaves_by_ids_not_available(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_try_select_leaves() {
        shared_tests::test_try_select_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_cancel_reservation() {
        shared_tests::test_cancel_reservation(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_cancel_reservation_drops_unkept_leaves() {
        shared_tests::test_cancel_reservation_drops_unkept_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_cancel_reservation_drops_all_when_keep_empty() {
        shared_tests::test_cancel_reservation_drops_all_when_keep_empty(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_cancel_reservation_nonexistent() {
        shared_tests::test_cancel_reservation_nonexistent(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_cancel_reservation_nonexistent_keeps_leaves() {
        shared_tests::test_cancel_reservation_nonexistent_keeps_leaves(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_finalize_reservation() {
        shared_tests::test_finalize_reservation(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_finalize_reservation_nonexistent() {
        shared_tests::test_finalize_reservation_nonexistent(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_multiple_reservations() {
        shared_tests::test_multiple_reservations(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reservation_ids_are_unique() {
        shared_tests::test_reservation_ids_are_unique(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_non_reservable_leaves() {
        shared_tests::test_non_reservable_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reserve_leaves_empty() {
        shared_tests::test_reserve_leaves_empty(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_swap_reservation_included_in_balance() {
        shared_tests::test_swap_reservation_included_in_balance(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_payment_reservation_excluded_from_balance() {
        shared_tests::test_payment_reservation_excluded_from_balance(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_try_reserve_success() {
        shared_tests::test_try_reserve_success(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_try_reserve_insufficient_funds() {
        shared_tests::test_try_reserve_insufficient_funds(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_try_reserve_wait_for_pending() {
        shared_tests::test_try_reserve_wait_for_pending(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_try_reserve_fail_immediately_when_insufficient() {
        shared_tests::test_try_reserve_fail_immediately_when_insufficient(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_try_reserve_min_amount_with_leaves_above_individual_target() {
        shared_tests::test_try_reserve_min_amount_with_leaves_above_individual_target(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_try_reserve_min_amount_exact_denominations_above_individual() {
        shared_tests::test_try_reserve_min_amount_exact_denominations_above_individual(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_balance_change_notification() {
        shared_tests::test_balance_change_notification(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_pending_cleared_on_cancel() {
        shared_tests::test_pending_cleared_on_cancel(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_pending_cleared_on_finalize() {
        shared_tests::test_pending_cleared_on_finalize(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_notification_after_swap_with_exact_amount() {
        shared_tests::test_notification_after_swap_with_exact_amount(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_notification_on_pending_balance_change() {
        shared_tests::test_notification_on_pending_balance_change(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_spent_leaves_not_restored_by_set_leaves() {
        shared_tests::test_spent_leaves_not_restored_by_set_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_skips_chains_of_spent_leaves() {
        shared_tests::test_set_leaves_skips_chains_of_spent_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
        shared_tests::test_spent_ids_cleaned_up_when_no_longer_in_refresh(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_add_leaves_not_deleted_by_set_leaves() {
        shared_tests::test_add_leaves_not_deleted_by_set_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_old_leaves_deleted_by_set_leaves() {
        // The shared test uses future_refresh_start() to ensure leaves added
        // "now" are treated as old relative to the refresh start.
        shared_tests::test_old_leaves_deleted_by_set_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_change_leaves_from_swap_protected() {
        shared_tests::test_change_leaves_from_swap_protected(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_finalize_with_new_leaves_protected() {
        shared_tests::test_finalize_with_new_leaves_protected(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_add_leaves_clears_spent_status() {
        shared_tests::test_add_leaves_clears_spent_status(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_skipped_during_active_swap() {
        shared_tests::test_set_leaves_skipped_during_active_swap(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_skipped_after_swap_completes_during_refresh() {
        shared_tests::test_set_leaves_skipped_after_swap_completes_during_refresh(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_set_leaves_proceeds_after_swap_when_refresh_starts_later() {
        shared_tests::test_set_leaves_proceeds_after_swap_when_refresh_starts_later(
            &InMemoryTreeStore::new(),
        )
        .await;
    }

    #[async_test_all]
    async fn test_payment_reservation_does_not_block_set_leaves() {
        shared_tests::test_payment_reservation_does_not_block_set_leaves(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_update_reservation_basic() {
        shared_tests::test_update_reservation_basic(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_update_reservation_nonexistent() {
        shared_tests::test_update_reservation_nonexistent(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_update_reservation_clears_pending() {
        shared_tests::test_update_reservation_clears_pending(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_update_reservation_preserves_purpose() {
        shared_tests::test_update_reservation_preserves_purpose(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_get_leaves_not_available() {
        shared_tests::test_get_leaves_not_available(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_get_leaves_missing_operators_filters_spent() {
        shared_tests::test_get_leaves_missing_operators_filters_spent(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_missing_operators_replaced_on_set_leaves() {
        shared_tests::test_missing_operators_replaced_on_set_leaves(&InMemoryTreeStore::new())
            .await;
    }

    #[async_test_all]
    async fn test_reserve_with_none_target_reserves_all() {
        shared_tests::test_reserve_with_none_target_reserves_all(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_reserve_skips_non_available_leaves() {
        shared_tests::test_reserve_skips_non_available_leaves(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_add_leaves_empty_slice() {
        shared_tests::test_add_leaves_empty_slice(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_full_payment_cycle() {
        shared_tests::test_full_payment_cycle(&InMemoryTreeStore::new()).await;
    }

    #[async_test_all]
    async fn test_set_leaves_replaces_fully() {
        shared_tests::test_set_leaves_replaces_fully(&InMemoryTreeStore::new()).await;
    }
}
