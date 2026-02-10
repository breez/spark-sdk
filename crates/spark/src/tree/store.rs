use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio_with_wasm::alias as tokio;
use tokio_with_wasm::alias::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tracing::{trace, warn};
use uuid::Uuid;

use crate::tree::{
    Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose, ReserveResult,
    TargetAmounts, TreeNode, TreeNodeId, TreeNodeStatus, TreeServiceError, TreeStore,
    select_helper,
};

/// Default maximum number of concurrent reservations allowed.
pub const DEFAULT_MAX_CONCURRENT_RESERVATIONS: usize = 30;

/// Default timeout for acquiring a reservation permit.
pub const DEFAULT_RESERVATION_TIMEOUT: Duration = Duration::from_secs(60);

/// Entry in the reservation map, containing leaves, purpose, and the semaphore permit.
/// The permit is automatically released when this entry is dropped.
struct ReservationEntry {
    leaves: Vec<TreeNode>,
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
    leaves: HashMap<TreeNodeId, TreeNode>,
    missing_operators_leaves: HashMap<TreeNodeId, TreeNode>,
    leaves_reservations: HashMap<LeavesReservationId, ReservationEntry>,
    /// Leaf IDs that have been finalized (spent). Prevents re-adding during refresh.
    spent_leaf_ids: HashSet<TreeNodeId>,
}

impl LeavesState {
    /// Calculate the available balance (unreserved available leaves).
    fn available_balance(&self) -> u64 {
        self.leaves
            .values()
            .filter(|leaf| leaf.status == TreeNodeStatus::Available)
            .map(|leaf| leaf.value)
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
    GetLeaves {
        response_tx: oneshot::Sender<Result<Leaves, TreeServiceError>>,
    },
    SetLeaves {
        leaves: Vec<TreeNode>,
        missing_operators_leaves: Vec<TreeNode>,
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
    #[cfg(test)]
    GetReservation {
        id: LeavesReservationId,
        response_tx: oneshot::Sender<Option<Vec<TreeNode>>>,
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
    balance_changed_rx: watch::Receiver<u64>,
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
        let (balance_changed_tx, balance_changed_rx) = watch::channel(0u64);
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
        balance_changed_tx: watch::Sender<u64>,
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
                StoreCommand::GetLeaves { response_tx } => {
                    let result = Self::process_get_leaves(&state);
                    let _ = response_tx.send(result);
                }
                StoreCommand::SetLeaves {
                    leaves,
                    missing_operators_leaves,
                    response_tx,
                } => {
                    let result =
                        Self::process_set_leaves(&mut state, &leaves, &missing_operators_leaves);
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
                StoreCommand::CancelReservation { id, response_tx } => {
                    // Permit is automatically released when ReservationEntry is dropped
                    Self::process_cancel_reservation(&mut state, &id);
                    let _ = response_tx.send(Ok(()));
                }
                StoreCommand::FinalizeReservation {
                    id,
                    new_leaves,
                    response_tx,
                } => {
                    // Permit is automatically released when ReservationEntry is dropped
                    Self::process_finalize_reservation(&mut state, &id, new_leaves.as_deref());
                    let _ = response_tx.send(Ok(()));
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
                #[cfg(test)]
                StoreCommand::GetReservation { id, response_tx } => {
                    let result = state
                        .leaves_reservations
                        .get(&id)
                        .map(|entry| entry.leaves.clone());
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
                let _ = balance_changed_tx.send(balance_after);
            }
        }
    }

    fn process_add_leaves(
        state: &mut LeavesState,
        leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        state
            .leaves
            .extend(leaves.iter().map(|l| (l.id.clone(), l.clone())));
        Ok(())
    }

    fn process_get_leaves(state: &LeavesState) -> Result<Leaves, TreeServiceError> {
        // Separate reserved leaves by purpose
        let mut reserved_for_payment = Vec::new();
        let mut reserved_for_swap = Vec::new();
        for entry in state.leaves_reservations.values() {
            match entry.purpose {
                ReservationPurpose::Payment => {
                    reserved_for_payment.extend(entry.leaves.iter().cloned());
                }
                ReservationPurpose::Swap => {
                    reserved_for_swap.extend(entry.leaves.iter().cloned());
                }
            }
        }

        Ok(Leaves {
            available: state
                .leaves
                .values()
                .filter(|leaf| leaf.status == TreeNodeStatus::Available)
                .cloned()
                .collect(),
            not_available: state
                .leaves
                .values()
                .filter(|leaf| leaf.status != TreeNodeStatus::Available)
                .cloned()
                .collect(),
            available_missing_from_operators: state
                .missing_operators_leaves
                .values()
                .filter(|leaf| leaf.status == TreeNodeStatus::Available)
                .cloned()
                .collect(),
            reserved_for_payment,
            reserved_for_swap,
        })
    }

    fn process_set_leaves(
        state: &mut LeavesState,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        // Collect IDs from the new refresh to clean up stale spent entries
        let refreshed_ids: HashSet<TreeNodeId> = leaves
            .iter()
            .chain(missing_operators_leaves.iter())
            .map(|l| l.id.clone())
            .collect();

        // Remove spent IDs that are no longer in the refresh (already gone from operators)
        state.spent_leaf_ids.retain(|id| refreshed_ids.contains(id));

        // Filter out leaves that were spent since refresh started
        state.leaves = leaves
            .iter()
            .filter(|l| !state.spent_leaf_ids.contains(&l.id))
            .map(|l| (l.id.clone(), l.clone()))
            .collect();

        state.missing_operators_leaves = missing_operators_leaves
            .iter()
            .filter(|l| !state.spent_leaf_ids.contains(&l.id))
            .map(|l| (l.id.clone(), l.clone()))
            .collect();

        for (key, entry) in state.leaves_reservations.iter_mut() {
            // Try to update reserved leaves with fresh data from the pool.
            // If a leaf is no longer in the pool (e.g., being swapped), keep the original.
            // IMPORTANT: Never remove reservations here - they might be in the middle of a swap
            // where leaves have been transferred but the swap hasn't completed yet.
            // Reservations should only be removed by explicit cancel/finalize.

            // Update leaves that exist in the refreshed pool
            for l in entry.leaves.iter_mut() {
                if let Some(leaf) = state.leaves.remove(&l.id) {
                    *l = leaf;
                } else if let Some(leaf) = state.missing_operators_leaves.remove(&l.id) {
                    *l = leaf;
                }
                // If leaf is not in either pool, keep the original (it might be in-flight)
            }

            trace!(
                "Updated reservation {}: refreshed {} leaves",
                key,
                entry.leaves.len()
            );
        }
        trace!(
            "Updated {:?} leaves in the local state (filtered {} spent)",
            state.leaves.len(),
            refreshed_ids
                .len()
                .saturating_sub(state.leaves.len() + state.missing_operators_leaves.len())
        );
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

        // Filter available leaves
        let leaves: Vec<TreeNode> = state
            .leaves
            .values()
            .filter(|leaf| leaf.status == TreeNodeStatus::Available)
            .cloned()
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

    /// Cancel a reservation and return leaves to the pool.
    /// The permit is automatically released when the ReservationEntry is dropped.
    fn process_cancel_reservation(state: &mut LeavesState, id: &LeavesReservationId) {
        // Return leaves to pool - permit and pending change are dropped with the entry
        if let Some(entry) = state.leaves_reservations.remove(id) {
            for leaf in entry.leaves {
                state.leaves.insert(leaf.id.clone(), leaf);
            }
            trace!("Canceled leaves reservation: {}", id);
        }
    }

    /// Finalize a reservation (leaves are consumed) and optionally add new leaves.
    /// The permit is automatically released when the ReservationEntry is dropped.
    fn process_finalize_reservation(
        state: &mut LeavesState,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) {
        // Remove reservation and record spent leaf IDs
        if let Some(entry) = state.leaves_reservations.remove(id) {
            // Mark all leaves from this reservation as spent to prevent re-adding during refresh
            for leaf in &entry.leaves {
                state.spent_leaf_ids.insert(leaf.id.clone());
            }
        } else {
            warn!("Tried to finalize a non existing reservation");
        }

        // Add new leaves (e.g., change from swap)
        if let Some(resulting_leaves) = new_leaves {
            state
                .leaves
                .extend(resulting_leaves.iter().map(|l| (l.id.clone(), l.clone())));
        }
        trace!("Finalized leaves reservation: {}", id);
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

        // Add change leaves to the available pool
        state
            .leaves
            .extend(change_leaves.iter().map(|l| (l.id.clone(), l.clone())));

        // Re-insert the reservation with updated leaves but same permit
        // Pending change is cleared since the swap completed
        let reserved = reserved_leaves.to_vec();
        state.leaves_reservations.insert(
            reservation_id.clone(),
            ReservationEntry {
                leaves: reserved.clone(),
                purpose,
                _permit: permit,
                pending_change_amount: 0,
            },
        );

        trace!(
            "Updated reservation {}: reserved {} leaves, added {} change leaves to pool",
            reservation_id,
            reserved.len(),
            change_leaves.len()
        );

        Ok(LeavesReservation::new(reserved, reservation_id.clone()))
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
        state.leaves_reservations.insert(
            id.clone(),
            ReservationEntry {
                leaves: leaves.to_vec(),
                purpose,
                _permit: permit,
                pending_change_amount,
            },
        );
        for leaf in leaves {
            state.leaves.remove(&leaf.id);
        }
        trace!("New leaves reservation {}: {:?}", id, leaves);
        Ok(id)
    }

    #[cfg(test)]
    async fn get_reservation(&self, id: &LeavesReservationId) -> Option<Vec<TreeNode>> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::GetReservation {
                id: id.clone(),
                response_tx,
            })
            .await
            .ok()?;
        response_rx.await.ok()?
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
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::AddLeaves {
                leaves: leaves.to_vec(),
                response_tx,
            })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::GetLeaves { response_tx })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }

    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::SetLeaves {
                leaves: leaves.to_vec(),
                missing_operators_leaves: missing_operators_leaves.to_vec(),
                response_tx,
            })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }

    async fn cancel_reservation(&self, id: &LeavesReservationId) -> Result<(), TreeServiceError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::CancelReservation {
                id: id.clone(),
                response_tx,
            })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::FinalizeReservation {
                id: id.clone(),
                new_leaves: new_leaves.map(|l| l.to_vec()),
                response_tx,
            })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
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

        let permit = tokio_with_wasm::alias::time::timeout(
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

        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::TryReserveLeaves {
                target_amounts: target_amounts.cloned(),
                exact_only,
                purpose,
                permit,
                response_tx,
            })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;

        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }

    fn subscribe_balance_changes(&self) -> watch::Receiver<u64> {
        self.balance_changed_rx.clone()
    }

    async fn update_reservation(
        &self,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(StoreCommand::UpdateReservation {
                reservation_id: reservation_id.clone(),
                reserved_leaves: reserved_leaves.to_vec(),
                change_leaves: change_leaves.to_vec(),
                response_tx,
            })
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?;
        response_rx
            .await
            .map_err(|_| TreeServiceError::ProcessorShutdown)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::ReservationPurpose;
    use bitcoin::{Transaction, absolute::LockTime, secp256k1::PublicKey, transaction::Version};
    use frost_secp256k1_tr::Identifier;
    use macros::async_test_all;
    use std::str::FromStr;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_tree_node(id: &str, value: u64) -> TreeNode {
        TreeNode {
            id: TreeNodeId::from_str(id).unwrap(),
            tree_id: "test_tree".to_string(),
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
            verifying_public_key: PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
            owner_identity_public_key: PublicKey::from_str(
                "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
            )
            .unwrap(),
            signing_keyshare: crate::tree::SigningKeyshare {
                public_key: PublicKey::from_str(
                    "02e6642fd69bd211f93f7f1f36ca51a26a5290eb2dd1b0d8279a87bb0d480c8443",
                )
                .unwrap(),
                owner_identifiers: vec![Identifier::try_from(1u16).unwrap()],
                threshold: 2,
            },
            status: crate::tree::TreeNodeStatus::Available,
        }
    }

    /// Helper function to reserve leaves in tests.
    /// Wraps try_reserve_leaves and expects success.
    async fn reserve_leaves(
        state: &InMemoryTreeStore,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError> {
        match state
            .try_reserve_leaves(target_amounts, exact_only, purpose)
            .await?
        {
            ReserveResult::Success(reservation) => Ok(reservation),
            ReserveResult::InsufficientFunds => Err(TreeServiceError::InsufficientFunds),
            ReserveResult::WaitForPending { .. } => Err(TreeServiceError::Generic(
                "Unexpected WaitForPending".into(),
            )),
        }
    }

    #[async_test_all]
    async fn test_new() {
        let state: InMemoryTreeStore = InMemoryTreeStore::new();
        assert!(state.get_leaves().await.unwrap().available.is_empty());
    }

    #[async_test_all]
    async fn test_add_leaves() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];

        state.add_leaves(&leaves).await.unwrap();

        let stored_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 2);
        assert!(
            stored_leaves
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 100)
        );
        assert!(
            stored_leaves
                .iter()
                .any(|l| l.id.to_string() == "node2" && l.value == 200)
        );
    }

    #[async_test_all]
    async fn test_add_leaves_duplicate_ids() {
        let state = InMemoryTreeStore::new();
        let leaf1 = create_test_tree_node("node1", 100);
        let leaf2 = create_test_tree_node("node1", 200); // Same ID, different value

        state.add_leaves(&[leaf1]).await.unwrap();
        state.add_leaves(&[leaf2]).await.unwrap();

        let stored_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 1);
        // Should have the second value (200) as it overwrites the first
        assert_eq!(stored_leaves[0].value, 200);
    }

    #[async_test_all]
    async fn test_set_leaves() {
        let state = InMemoryTreeStore::new();
        let initial_leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&initial_leaves).await.unwrap();

        let new_leaves = vec![
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.set_leaves(&new_leaves, &[]).await.unwrap();

        let stored_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(stored_leaves.len(), 2);
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node2"));
        assert!(stored_leaves.iter().any(|l| l.id.to_string() == "node3"));
        assert!(!stored_leaves.iter().any(|l| l.id.to_string() == "node1"));
    }

    #[async_test_all]
    async fn test_set_leaves_with_reservations() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves
        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(600, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Update leaves with new data (including updated versions of reserved leaves)
        let non_existing_operator_leaf = create_test_tree_node("node7", 1000); // Updated value
        let mut updated_leaf1 = create_test_tree_node("node1", 150); // Updated value
        updated_leaf1.status = crate::tree::TreeNodeStatus::TransferLocked;
        let new_leaves = vec![
            updated_leaf1,
            create_test_tree_node("node2", 250), // Updated value
            create_test_tree_node("node4", 400), // New leaf, node3 removed
        ];
        state
            .set_leaves(&new_leaves, &[non_existing_operator_leaf])
            .await
            .unwrap();

        // Check that reserved leaves were updated with new data where available.
        // With the new behavior, reservations are NEVER removed by set_leaves.
        // Leaves that exist in the pool are updated; others keep their original values.
        let reservation = state.get_reservation(&reservation.id).await.unwrap();
        // All 3 original leaves are preserved (node3 keeps original value since not in pool)
        assert_eq!(reservation.len(), 3);
        // Find each leaf and verify
        let node1 = reservation
            .iter()
            .find(|l| l.id.to_string() == "node1")
            .unwrap();
        let node2 = reservation
            .iter()
            .find(|l| l.id.to_string() == "node2")
            .unwrap();
        let node3 = reservation
            .iter()
            .find(|l| l.id.to_string() == "node3")
            .unwrap();
        assert_eq!(node1.value, 150); // Updated
        assert_eq!(node1.status, crate::tree::TreeNodeStatus::TransferLocked);
        assert_eq!(node2.value, 250); // Updated
        assert_eq!(node3.value, 300); // Original (not in new pool)

        // Check main pool
        let all_leaves = state.get_leaves().await.unwrap();
        // Reserved balance is now 150 + 250 + 300 = 700 (but node3 was updated before reservation)
        // Actually the original test reserved all 3 nodes (100+200+300=600)
        // After set_leaves, the reservation has updated values: 150+250+300=700
        assert_eq!(all_leaves.payment_reserved_balance(), 700);
        assert_eq!(all_leaves.available_balance(), 400);
        assert_eq!(all_leaves.missing_operators_balance(), 1000);
        // balance() excludes payment-reserved leaves
        assert_eq!(all_leaves.balance(), 400 + 1000);
        assert_eq!(all_leaves.available.len(), 1); // Only node4 should be in main pool
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node4")
        );
    }

    #[async_test_all]
    async fn test_set_leaves_preserves_reservations_for_in_flight_swaps() {
        // Test that reservations are preserved even when leaves are no longer in the pool.
        // This is important for swaps where leaves are transferred but the swap hasn't completed.
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve leaves (simulating start of a swap)
        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            false,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Set new leaves that don't include the reserved ones
        // (simulating a refresh while swap is in progress)
        let new_leaves = vec![create_test_tree_node("node3", 300)];
        state.set_leaves(&new_leaves, &[]).await.unwrap();

        // Reservation should be PRESERVED (not removed) - the swap might still complete
        let reserved = state.get_reservation(&reservation.id).await;
        assert!(reserved.is_some());
        // The reserved leaves keep their original values since they're not in the new pool
        let reserved = reserved.unwrap();
        assert_eq!(reserved.len(), 2);
        assert!(
            reserved
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 100)
        );
        assert!(
            reserved
                .iter()
                .any(|l| l.id.to_string() == "node2" && l.value == 200)
        );
    }

    #[async_test_all]
    async fn test_reserve_leaves() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Check that reservation was created
        let reserved = state.get_reservation(&reservation.id).await.unwrap();
        assert_eq!(reserved.len(), 1);
        assert_eq!(reserved[0].id, leaves[0].id);
        // Check that leaf was removed from main pool
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[async_test_all]
    async fn test_cancel_reservation() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Cancel the reservation
        state.cancel_reservation(&reservation.id).await.unwrap();

        // Check that reservation was removed
        assert!(state.get_reservation(&reservation.id).await.is_none());

        // Check that leaf was returned to main pool
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 2);
        assert!(main_leaves.iter().any(|l| l.id == leaves[0].id));
        assert!(main_leaves.iter().any(|l| l.id == leaves[1].id));
    }

    #[async_test_all]
    async fn test_cancel_reservation_nonexistent() {
        let state = InMemoryTreeStore::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.cancel_reservation(&fake_id).await.unwrap();

        let main_leaves = state.get_leaves().await.unwrap().available;
        assert!(main_leaves.is_empty());
    }

    #[async_test_all]
    async fn test_finalize_reservation() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Finalize the reservation
        state
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // Check that reservation was removed
        assert!(state.get_reservation(&reservation.id).await.is_none());

        // Check that leaf was NOT returned to main pool (it's considered used)
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[1].id);
    }

    #[async_test_all]
    async fn test_finalize_reservation_nonexistent() {
        let state = InMemoryTreeStore::new();
        let fake_id = "fake-reservation-id".to_string();

        // Should not panic or cause issues
        state.finalize_reservation(&fake_id, None).await.unwrap();

        let main_leaves = state.get_leaves().await.unwrap().available;
        assert!(main_leaves.is_empty());
    }

    #[async_test_all]
    async fn test_multiple_reservations() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Create multiple reservations
        let reservation1 = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        let reservation2 = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(200, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Check both reservations exist
        assert!(state.get_reservation(&reservation1.id).await.is_some());
        assert!(state.get_reservation(&reservation2.id).await.is_some());
        assert_eq!(
            state.get_reservation(&reservation1.id).await.unwrap().len(),
            1
        );
        assert_eq!(
            state.get_reservation(&reservation2.id).await.unwrap().len(),
            1
        );

        // Check main pool has only one leaf left
        let main_leaves = state.get_leaves().await.unwrap().available;
        assert_eq!(main_leaves.len(), 1);
        assert_eq!(main_leaves[0].id, leaves[2].id);

        // Cancel one reservation
        state.cancel_reservation(&reservation1.id).await.unwrap();
        assert!(state.get_reservation(&reservation1.id).await.is_none());
        assert_eq!(state.get_leaves().await.unwrap().available.len(), 2);

        // Finalize the other
        state
            .finalize_reservation(&reservation2.id, None)
            .await
            .unwrap();
        assert!(state.get_reservation(&reservation2.id).await.is_none());
        assert_eq!(state.get_leaves().await.unwrap().available.len(), 2); // node1 returned, node3 was always there
    }

    #[async_test_all]
    async fn test_reservation_ids_are_unique() {
        let state = InMemoryTreeStore::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

        let r1 = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        state.cancel_reservation(&r1.id).await.unwrap();
        let r2 = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        assert_ne!(r1.id, r2.id);
    }

    #[async_test_all]
    async fn test_non_reservable_leaves() {
        let state = InMemoryTreeStore::new();
        let leaf = create_test_tree_node("node1", 100);
        state.add_leaves(std::slice::from_ref(&leaf)).await.unwrap();

        reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        let result = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap_err();
        assert!(matches!(result, TreeServiceError::InsufficientFunds));
    }

    #[async_test_all]
    async fn test_reserve_leaves_empty() {
        let state = InMemoryTreeStore::new();
        let err = reserve_leaves(&state, None, false, ReservationPurpose::Payment)
            .await
            .unwrap_err();

        assert!(matches!(err, TreeServiceError::NonReservableLeaves));
    }

    #[async_test_all]
    async fn test_swap_reservation_included_in_balance() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves for swap
        let _reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            true,
            ReservationPurpose::Swap,
        )
        .await
        .unwrap();

        // Check that swap-reserved leaves are included in balance
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.swap_reserved_balance(), 300);
        assert_eq!(all_leaves.available_balance(), 300); // node1 + node2 remaining
        // balance() should include swap-reserved leaves
        assert_eq!(all_leaves.balance(), 300 + 300); // available + swap-reserved
    }

    #[async_test_all]
    async fn test_payment_reservation_excluded_from_balance() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve some leaves for payment
        let _reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(300, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Check that payment-reserved leaves are excluded from balance
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.payment_reserved_balance(), 300);
        assert_eq!(all_leaves.available_balance(), 300); // node1 + node2 remaining
        // balance() should NOT include payment-reserved leaves
        assert_eq!(all_leaves.balance(), 300); // only available
    }

    // Tests for try_reserve_leaves and balance notifications

    #[async_test_all]
    async fn test_try_reserve_success() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        let result = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                true,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert!(matches!(result, ReserveResult::Success(_)));
        if let ReserveResult::Success(reservation) = result {
            assert_eq!(reservation.sum(), 100);
        }
    }

    #[async_test_all]
    async fn test_try_reserve_insufficient_funds() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&leaves).await.unwrap();

        let result = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(500, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert!(matches!(result, ReserveResult::InsufficientFunds));
    }

    #[async_test_all]
    async fn test_try_reserve_wait_for_pending() {
        let state = InMemoryTreeStore::new();
        // Add a single 1000 sat leaf
        let leaves = vec![create_test_tree_node("node1", 1000)];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve with target 100 - store will reserve 1000 and auto-track pending=900
        let r1 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        assert!(matches!(r1, ReserveResult::Success(_)));

        // Try to reserve 300 more - should get WaitForPending since pending=900 > 300
        let r2 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        match r2 {
            ReserveResult::WaitForPending {
                needed,
                available,
                pending,
            } => {
                assert_eq!(needed, 300);
                assert_eq!(available, 0);
                assert_eq!(pending, 900);
            }
            _ => panic!("Expected WaitForPending, got {:?}", r2),
        }
    }

    #[async_test_all]
    async fn test_try_reserve_fail_immediately_when_insufficient() {
        let state = InMemoryTreeStore::new();
        // Add 100 sat leaf
        let leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve it for 50 sats - pending will be 50
        let r1 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(50, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        assert!(matches!(r1, ReserveResult::Success(_)));

        // Request 500 - more than available + pending (0 + 50 < 500)
        let result = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(500, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();
        assert!(matches!(result, ReserveResult::InsufficientFunds));
    }

    #[async_test_all]
    async fn test_balance_change_notification() {
        let state = InMemoryTreeStore::new();
        let mut rx = state.subscribe_balance_changes();

        // Add leaves
        let leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&leaves).await.unwrap();

        // Wait for notification with timeout
        let result =
            tokio_with_wasm::alias::time::timeout(std::time::Duration::from_millis(100), async {
                rx.changed().await.ok();
                *rx.borrow()
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100);
    }

    #[async_test_all]
    async fn test_pending_cleared_on_cancel() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![create_test_tree_node("node1", 1000)];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve with target 100 - auto-tracks pending=900
        let r1 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Cancel the reservation - pending should be cleared
        state.cancel_reservation(&reservation_id).await.unwrap();

        // Try to reserve 300 - should succeed since 1000 sat leaf is back
        let r2 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Now 1000 sat leaf is back, so we should succeed
        assert!(matches!(r2, ReserveResult::Success(_)));
    }

    #[async_test_all]
    async fn test_pending_cleared_on_finalize() {
        let state = InMemoryTreeStore::new();
        let leaves = vec![create_test_tree_node("node1", 1000)];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve with target 100 - auto-tracks pending=900
        let r1 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Finalize with new leaves (the change from swap)
        let change_leaf = create_test_tree_node("node2", 900);
        state
            .finalize_reservation(&reservation_id, Some(&[change_leaf]))
            .await
            .unwrap();

        // Try to reserve 300 - should succeed since change is now available
        let r2 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(300, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        assert!(matches!(r2, ReserveResult::Success(_)));
    }

    #[async_test_all]
    async fn test_notification_after_swap_with_exact_amount() {
        // This test verifies that waiters are notified even when a swap returns
        // exactly the target amount (net balance doesn't change)
        let state = InMemoryTreeStore::new();
        let mut rx = state.subscribe_balance_changes();

        // Add a single 1000 sat leaf
        let leaves = vec![create_test_tree_node("node1", 1000)];
        state.add_leaves(&leaves).await.unwrap();

        // Consume the initial notification
        let _ = tokio_with_wasm::alias::time::timeout(
            std::time::Duration::from_millis(100),
            rx.changed(),
        )
        .await;

        // Reserve it with target 100 - will reserve all 1000, pending=900
        let r1 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Consume the reservation notification
        let _ = tokio_with_wasm::alias::time::timeout(
            std::time::Duration::from_millis(100),
            rx.changed(),
        )
        .await;

        // Simulate a swap that returns exactly the target amount (100 sats)
        // This is the scenario that was causing hanging - balance goes 0 -> 100 -> 0
        let swap_result_leaf = create_test_tree_node("node2", 100);
        // reserved_leaves: the leaf we want to keep (100 sats)
        // change_leaves: empty (no change since swap returned exact amount)
        state
            .update_reservation(&reservation_id, &[swap_result_leaf], &[])
            .await
            .unwrap();

        // Verify that we still get a notification even though net balance is 0 -> 0
        let notification_result = tokio_with_wasm::alias::time::timeout(
            std::time::Duration::from_millis(100),
            rx.changed(),
        )
        .await;

        // The notification should be received (not timeout)
        assert!(
            notification_result.is_ok(),
            "Expected notification after swap update with exact amount"
        );
    }

    #[async_test_all]
    async fn test_notification_on_pending_balance_change() {
        // Test that notifications are sent when pending balance changes
        let state = InMemoryTreeStore::new();
        let mut rx = state.subscribe_balance_changes();

        // Add a single 1000 sat leaf
        let leaves = vec![create_test_tree_node("node1", 1000)];
        state.add_leaves(&leaves).await.unwrap();

        // Consume initial notification
        let _ = tokio_with_wasm::alias::time::timeout(
            std::time::Duration::from_millis(100),
            rx.changed(),
        )
        .await;

        // Reserve with target 100 - pending=900 (since we reserved 1000 for 100 target)
        let r1 = state
            .try_reserve_leaves(
                Some(&TargetAmounts::new_amount_and_fee(100, None)),
                false,
                ReservationPurpose::Payment,
            )
            .await
            .unwrap();

        // Consume reservation notification
        let _ = tokio_with_wasm::alias::time::timeout(
            std::time::Duration::from_millis(100),
            rx.changed(),
        )
        .await;

        let reservation_id = match r1 {
            ReserveResult::Success(r) => r.id,
            _ => panic!("Expected Success"),
        };

        // Cancel the reservation - this clears pending from 900 to 0
        // Even though available balance goes back to 1000, pending changed too
        state.cancel_reservation(&reservation_id).await.unwrap();

        // Should get notification because pending balance changed
        let notification_result = tokio_with_wasm::alias::time::timeout(
            std::time::Duration::from_millis(100),
            rx.changed(),
        )
        .await;

        assert!(
            notification_result.is_ok(),
            "Expected notification when pending balance changes"
        );
    }

    #[async_test_all]
    async fn test_spent_leaves_not_restored_by_set_leaves() {
        // Test that finalized (spent) leaves are not restored when set_leaves is called
        // with stale data from operators. This prevents the TOCTOU race condition where
        // a refresh started before a payment completes would re-add spent leaves.
        let state = InMemoryTreeStore::new();
        let leaves = vec![
            create_test_tree_node("node1", 100),
            create_test_tree_node("node2", 200),
        ];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve node1 for payment
        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();

        // Finalize the reservation (node1 is now spent)
        state
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // Verify node1 is not in the pool
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1")
        );

        // Now simulate a refresh that returns stale data including the spent leaf
        // (this is the race condition scenario - refresh started before finalize completed)
        let stale_leaves = vec![
            create_test_tree_node("node1", 100), // This was spent!
            create_test_tree_node("node2", 200),
            create_test_tree_node("node3", 300), // New leaf
        ];
        state.set_leaves(&stale_leaves, &[]).await.unwrap();

        // Verify node1 was NOT restored (it's in spent_leaf_ids)
        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 2); // node2 and node3 only
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node3")
        );
        assert!(
            !all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1"),
            "Spent leaf node1 should not be restored by set_leaves"
        );
    }

    #[async_test_all]
    async fn test_spent_ids_cleaned_up_when_no_longer_in_refresh() {
        // Test that spent_leaf_ids are cleaned up when the operators no longer
        // return those leaves (they've been fully processed on the operator side)
        let state = InMemoryTreeStore::new();
        let leaves = vec![create_test_tree_node("node1", 100)];
        state.add_leaves(&leaves).await.unwrap();

        // Reserve and finalize node1
        let reservation = reserve_leaves(
            &state,
            Some(&TargetAmounts::new_amount_and_fee(100, None)),
            true,
            ReservationPurpose::Payment,
        )
        .await
        .unwrap();
        state
            .finalize_reservation(&reservation.id, None)
            .await
            .unwrap();

        // First refresh still includes node1 (stale) - should be filtered
        let stale_leaves = vec![create_test_tree_node("node1", 100)];
        state.set_leaves(&stale_leaves, &[]).await.unwrap();
        assert!(state.get_leaves().await.unwrap().available.is_empty());

        // Second refresh no longer includes node1 (operators caught up)
        // The spent_leaf_ids entry should be cleaned up
        let fresh_leaves = vec![create_test_tree_node("node2", 200)];
        state.set_leaves(&fresh_leaves, &[]).await.unwrap();

        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 1);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node2")
        );

        // Now if a new node1 appears (different transaction, same ID pattern is unlikely
        // but tests the cleanup), it should be accepted since it's no longer in spent_leaf_ids
        let new_node1_leaves = vec![
            create_test_tree_node("node1", 150), // New node1, different value
            create_test_tree_node("node2", 200),
        ];
        state.set_leaves(&new_node1_leaves, &[]).await.unwrap();

        let all_leaves = state.get_leaves().await.unwrap();
        assert_eq!(all_leaves.available.len(), 2);
        assert!(
            all_leaves
                .available
                .iter()
                .any(|l| l.id.to_string() == "node1" && l.value == 150)
        );
    }
}
