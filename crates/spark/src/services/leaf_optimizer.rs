use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, trace, warn};

use crate::{
    services::{ServiceError, Swap},
    tree::{LeavesReservationId, ReservationPurpose, TreeNode, TreeService},
};

/// Default maximum number of leaves per swap round
pub const DEFAULT_MAX_LEAVES_PER_SWAP: u32 = 64;

/// Configuration options for leaf optimization.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OptimizationOptions {
    /// Whether optimization should run automatically after sync/receive operations.
    pub auto_enabled: bool,
    /// Controls the optimization aggressiveness. Higher values create more leaves
    /// for flexibility but may slow down operations. Recommended: 1 or 2.
    pub multiplicity: u8,
    /// Soft limit on the number of leaves per swap round.
    pub max_leaves_per_swap: u32,
}

impl Default for OptimizationOptions {
    fn default() -> Self {
        Self {
            auto_enabled: true,
            multiplicity: 2,
            max_leaves_per_swap: DEFAULT_MAX_LEAVES_PER_SWAP,
        }
    }
}

impl OptimizationOptions {
    pub fn validate(&self) -> Result<(), ServiceError> {
        if self.multiplicity > 5 {
            return Err(ServiceError::Generic(
                "Multiplicity cannot be greater than 5".to_string(),
            ));
        }
        if self.max_leaves_per_swap == 0 {
            return Err(ServiceError::Generic(
                "max_leaves_per_swap must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// A snapshot of the current optimization progress.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct OptimizationProgress {
    /// Whether optimization is currently running.
    pub is_running: bool,
    /// The current round being executed (1-indexed when running).
    pub current_round: u32,
    /// The total number of rounds to execute.
    pub total_rounds: u32,
}

/// Events emitted during optimization lifecycle.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum OptimizationEvent {
    /// Optimization has started with the given number of rounds.
    Started { total_rounds: u32 },
    /// A round has completed.
    RoundCompleted {
        current_round: u32,
        total_rounds: u32,
    },
    /// Optimization completed successfully.
    Completed,
    /// Optimization was cancelled by the user.
    Cancelled,
    /// Optimization failed with an error.
    Failed { error: String },
    /// Optimization was skipped because leaves are already optimal.
    Skipped,
}

/// Trait for receiving optimization events.
/// Implemented by the wallet layer to convert to WalletEvents.
pub trait OptimizationEventHandler: Send + Sync {
    fn on_optimization_event(&self, event: OptimizationEvent);
}

/// Represents a single swap operation in the optimization process.
#[derive(Clone, Debug)]
pub struct OptimizationSwap {
    /// The leaf values to give up in this swap.
    pub leaves_to_give: Vec<u64>,
    /// The leaf values to receive in this swap.
    pub leaves_to_receive: Vec<u64>,
}

/// Service responsible for optimizing leaf denominations.
///
/// The optimizer transforms the current set of leaves into an optimal set
/// that minimizes the probability of needing swaps during transfers.
/// It operates in multiple rounds, each performing a swap operation.
pub struct LeafOptimizer {
    config: OptimizationOptions,
    swap_service: Arc<Swap>,
    tree_service: Arc<dyn TreeService>,
    progress: Mutex<OptimizationProgress>,
    cancel_tx: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
    terminated: Notify,
    event_handler: Option<Arc<dyn OptimizationEventHandler>>,
}

impl LeafOptimizer {
    pub fn new(
        config: OptimizationOptions,
        swap_service: Arc<Swap>,
        tree_service: Arc<dyn TreeService>,
        event_handler: Option<Arc<dyn OptimizationEventHandler>>,
    ) -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        Self {
            config,
            swap_service,
            tree_service,
            progress: Mutex::new(OptimizationProgress::default()),
            cancel_tx,
            cancel_rx,
            terminated: Notify::new(),
            event_handler,
        }
    }

    /// Returns the current optimization progress snapshot.
    pub fn progress(&self) -> OptimizationProgress {
        self.progress.lock().unwrap().clone()
    }

    /// Checks if optimization is currently running and may have leaves in use.
    /// Used to determine if optimization should be cancelled when payments fail.
    pub fn has_reserved_leaves(&self) -> bool {
        self.progress.lock().unwrap().is_running
    }

    /// Static helper to check if leaves need optimization.
    pub async fn should_optimize(&self) -> Result<bool, ServiceError> {
        let leaves = self.tree_service.list_leaves().await?.available;
        let leave_amounts = leaves.iter().map(|leaf| leaf.value).collect::<Vec<u64>>();

        if self.config.multiplicity == 0 {
            // Optimize if it reduces the number of leaves by more than 5x
            let swaps = self.maximize_unilateral_exit(&leave_amounts);
            let num_inputs: usize = swaps.iter().map(|swap| swap.leaves_to_give.len()).sum();
            let num_outputs: usize = swaps.iter().map(|swap| swap.leaves_to_receive.len()).sum();
            Ok(num_outputs * 5 < num_inputs)
        } else {
            // Optimize if the number of input denominations differs from the number of output denominations by more than 2
            let swaps = self.minimize_transfer_swap(&leave_amounts);

            let input_counter = Self::count_occurrences(
                &swaps
                    .iter()
                    .flat_map(|swap| swap.leaves_to_give.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            );

            let output_counter = Self::count_occurrences(
                &swaps
                    .iter()
                    .flat_map(|swap| swap.leaves_to_receive.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            );

            Ok((input_counter.len() as i64 - output_counter.len() as i64).abs() > 2)
        }
    }

    /// Starts the optimization process in the background.
    ///
    /// This method spawns the optimization work in a background task and returns
    /// immediately. Progress is reported via events.
    ///
    /// Returns early (without spawning) if:
    /// - Optimization is already running
    pub async fn start(self: &Arc<Self>) -> Result<(), ServiceError> {
        // Check if already running
        if self.progress.lock().unwrap().is_running {
            debug!("Optimization already running, skipping");
            return Ok(());
        }

        // Spawn the optimization work in the background
        let optimizer = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = optimizer.run_optimization().await {
                error!("Optimization failed: {:?}", e);
            }
        });

        Ok(())
    }

    /// Internal method that runs the actual optimization logic.
    async fn run_optimization(&self) -> Result<(), ServiceError> {
        // Reset cancellation flag
        let _ = self.cancel_tx.send(false);

        // Reserve ALL available leaves for the duration of optimization.
        // Use Optimization purpose so these leaves are still counted in the balance.
        let reservation = self
            .tree_service
            .select_leaves(None, ReservationPurpose::Optimization)
            .await?;

        if reservation.leaves.is_empty() {
            debug!("No leaves available for optimization");
            self.emit_event(OptimizationEvent::Skipped);
            return Ok(());
        }

        // Calculate the swaps needed based on the reserved leaves
        let leaf_values: Vec<u64> = reservation.leaves.iter().map(|l| l.value).collect();
        let swaps = self.calculate_optimization_swaps(&leaf_values);

        if swaps.is_empty() {
            debug!("No swaps needed for optimization");
            let _ = self.tree_service.cancel_reservation(reservation.id).await;
            self.emit_event(OptimizationEvent::Skipped);
            return Ok(());
        }

        let total_rounds = swaps.len() as u32;

        // Mark as running
        {
            let mut progress = self.progress.lock().unwrap();
            *progress = OptimizationProgress {
                is_running: true,
                current_round: 0,
                total_rounds,
            };
        }

        self.emit_event(OptimizationEvent::Started { total_rounds });
        info!("Starting leaf optimization with {} rounds", total_rounds);

        // Execute each swap round using the reserved leaves
        let result = self
            .execute_optimization_rounds(swaps, reservation.leaves, reservation.id)
            .await;

        // Mark as stopped and notify any waiters (e.g., cancel method)
        {
            let mut progress = self.progress.lock().unwrap();
            *progress = OptimizationProgress::default();
        }
        self.terminated.notify_waiters();

        match result {
            Ok(true) => {
                info!("Leaf optimization completed successfully");
                self.emit_event(OptimizationEvent::Completed);
                Ok(())
            }
            Ok(false) => {
                info!("Leaf optimization was cancelled");
                self.emit_event(OptimizationEvent::Cancelled);
                Ok(())
            }
            Err(e) => {
                error!("Leaf optimization failed: {:?}", e);
                self.emit_event(OptimizationEvent::Failed {
                    error: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Cancels the ongoing optimization and waits for it to fully stop.
    ///
    /// This sets a cancellation flag that is checked between rounds.
    /// The current round will complete before stopping. This method blocks
    /// until the optimization has fully stopped and leaves are available again.
    pub async fn cancel(&self) -> Result<(), ServiceError> {
        // First check if optimization is running
        if !self.progress.lock().unwrap().is_running {
            debug!("No optimization running to cancel");
            return Ok(());
        }

        info!("Cancelling leaf optimization and waiting for completion");

        // Create the notified future BEFORE sending cancel signal to avoid race conditions
        let notified = self.terminated.notified();

        // Send cancel signal
        let _ = self.cancel_tx.send(true);

        // Double-check: if optimization already stopped between our first check
        // and creating the notified future, we can return early
        if !self.progress.lock().unwrap().is_running {
            debug!("Optimization already stopped");
            return Ok(());
        }

        // Wait for the termination signal
        notified.await;

        debug!("Optimization cancelled and stopped");
        Ok(())
    }

    /// Executes the optimization rounds.
    /// Returns Ok(true) if completed, Ok(false) if cancelled, Err on failure.
    ///
    /// Takes ownership of the reserved leaves and manages the reservation lifecycle:
    /// - On success: finalizes the reservation (removes old leaves from local store)
    /// - On cancellation/failure with no progress: cancels reservation (returns leaves to available)
    /// - On cancellation/failure with partial progress: refreshes leaves from server to sync state
    async fn execute_optimization_rounds(
        &self,
        swaps: Vec<OptimizationSwap>,
        mut available_leaves: Vec<TreeNode>,
        reservation_id: LeavesReservationId,
    ) -> Result<bool, ServiceError> {
        let total_rounds = swaps.len() as u32;
        let mut completed_rounds = 0u32;

        for (index, swap) in swaps.into_iter().enumerate() {
            let round = (index + 1) as u32;

            // Check for cancellation before each round
            if *self.cancel_rx.borrow() {
                debug!("Optimization cancelled before round {}", round);
                self.cleanup_after_interruption(reservation_id, completed_rounds)
                    .await;
                return Ok(false);
            }

            trace!(
                "Executing optimization round {}/{}: give {:?}, receive {:?}",
                round, total_rounds, swap.leaves_to_give, swap.leaves_to_receive
            );

            // Update progress with current round
            {
                let mut progress = self.progress.lock().unwrap();
                *progress = OptimizationProgress {
                    is_running: true,
                    current_round: round,
                    total_rounds,
                };
            }

            // Find leaves matching our swap from the reserved leaves
            let leaves_for_swap =
                self.select_leaves_for_swap(&available_leaves, &swap.leaves_to_give)?;

            if leaves_for_swap.is_empty() {
                warn!(
                    "Could not find matching leaves for optimization round {}",
                    round
                );
                continue;
            }

            // Execute the swap
            let target_amounts = if swap.leaves_to_receive.is_empty() {
                None
            } else {
                Some(swap.leaves_to_receive.clone())
            };

            match self
                .swap_service
                .swap_leaves(&leaves_for_swap, target_amounts)
                .await
            {
                Ok(new_leaves) => {
                    // Remove the swapped leaves from our working set so we don't
                    // try to use them in subsequent rounds. Note: we don't add the
                    // new leaves to the working set because swap rounds are independent -
                    // each round's output is NOT input for subsequent rounds.
                    let swapped_ids: Vec<_> = leaves_for_swap.iter().map(|l| &l.id).collect();
                    available_leaves.retain(|l| !swapped_ids.contains(&&l.id));

                    // Insert the new leaves immediately - they're available for payments
                    // even while optimization continues
                    if let Err(e) = self.tree_service.insert_leaves(new_leaves).await {
                        error!("Failed to insert optimized leaves: {:?}", e);
                        self.cleanup_after_interruption(reservation_id, completed_rounds)
                            .await;
                        return Err(ServiceError::Generic(format!(
                            "Failed to insert optimized leaves: {e:?}"
                        )));
                    }

                    completed_rounds = round;

                    self.emit_event(OptimizationEvent::RoundCompleted {
                        current_round: round,
                        total_rounds,
                    });

                    debug!("Completed optimization round {}/{}", round, total_rounds);
                }
                Err(e) => {
                    error!("Swap failed in optimization round {}: {:?}", round, e);
                    self.cleanup_after_interruption(reservation_id, completed_rounds)
                        .await;
                    return Err(e);
                }
            }
        }

        // All rounds completed successfully - finalize the reservation
        // This removes the original leaves from the local store
        // (they've already been consumed server-side by the swaps)
        if let Err(e) = self.tree_service.finalize_reservation(reservation_id).await {
            warn!("Failed to finalize optimization reservation: {:?}", e);
        }

        Ok(true)
    }

    /// Cleans up local state after optimization is interrupted (cancelled or failed).
    ///
    /// If no rounds completed, we can simply cancel the reservation to return leaves
    /// to the available pool. If some rounds completed, the local state is inconsistent
    /// (some reserved leaves no longer exist), so we refresh from the server.
    async fn cleanup_after_interruption(
        &self,
        reservation_id: LeavesReservationId,
        completed_rounds: u32,
    ) {
        if completed_rounds == 0 {
            // No swaps happened - safe to just cancel the reservation
            debug!("No rounds completed, cancelling reservation");
            let _ = self.tree_service.cancel_reservation(reservation_id).await;
        } else {
            // Some swaps happened - local state is inconsistent
            // The reservation contains stale leaf IDs that were already swapped.
            // Refresh from server to get the ground truth.
            debug!(
                "Optimization interrupted after {} rounds, refreshing leaves from server",
                completed_rounds
            );

            // First cancel the reservation to clear the stale reserved state
            let _ = self.tree_service.cancel_reservation(reservation_id).await;

            // Then refresh from server to sync with actual state
            if let Err(e) = self.tree_service.refresh_leaves().await {
                warn!(
                    "Failed to refresh leaves after optimization interruption: {:?}",
                    e
                );
            }
        }
    }

    /// Selects leaves from available leaves that match the target values.
    fn select_leaves_for_swap(
        &self,
        available_leaves: &[TreeNode],
        target_values: &[u64],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let mut selected = Vec::new();
        let mut remaining_values: Vec<u64> = target_values.to_vec();
        let mut available: Vec<&TreeNode> = available_leaves.iter().collect();

        for target in &mut remaining_values {
            if let Some(pos) = available.iter().position(|l| l.value == *target) {
                selected.push(available.remove(pos).clone());
            } else {
                // Could not find exact match - this might happen if leaves changed
                warn!("Could not find leaf with value {} for optimization", target);
            }
        }

        Ok(selected)
    }

    fn calculate_optimization_swaps(&self, input_leave_amounts: &[u64]) -> Vec<OptimizationSwap> {
        if self.config.multiplicity == 0 {
            self.maximize_unilateral_exit(input_leave_amounts)
        } else {
            self.minimize_transfer_swap(input_leave_amounts)
        }
    }

    /// Calculates the swaps needed to optimize when maximizing unilateral exit.
    ///
    /// Generates swaps that will result in the unilateral exit maximizing set of leaves.
    /// Multiple iterations may be required to reach the optimal set.
    fn maximize_unilateral_exit(&self, input_leave_amounts: &[u64]) -> Vec<OptimizationSwap> {
        let max_leaves_per_swap = self.config.max_leaves_per_swap as usize;
        let mut swaps = Vec::new();
        let mut batch: Vec<u64> = Vec::new();

        // Sort leaves ascending
        let mut leaves: Vec<u64> = input_leave_amounts.to_vec();
        leaves.sort();

        // Process leaves in batches of up to approximately max_leaves_per_swap
        while !leaves.is_empty() {
            batch.push(leaves.remove(0));
            let batch_sum: u64 = batch.iter().sum();
            let target = Self::greedy_leaves(batch_sum);

            if batch.len() >= max_leaves_per_swap || target.len() >= max_leaves_per_swap {
                if target != batch {
                    swaps.push(OptimizationSwap {
                        leaves_to_give: batch.clone(),
                        leaves_to_receive: target,
                    });
                }
                batch.clear();
            }
        }

        // Process any remaining leaves
        if !batch.is_empty() {
            let batch_sum: u64 = batch.iter().sum();
            let target = Self::greedy_leaves(batch_sum);

            if target != batch {
                swaps.push(OptimizationSwap {
                    leaves_to_give: batch,
                    leaves_to_receive: target,
                });
            }
        }

        swaps
    }

    /// Calculates the swaps needed to optimize when minimizing transfer swaps.
    ///
    /// Generates swaps that will minimize the probability of needing to swap during a transfer.
    /// Multiple iterations may be required to reach the optimal set.
    fn minimize_transfer_swap(&self, input_leave_amounts: &[u64]) -> Vec<OptimizationSwap> {
        let max_leaves = self.config.max_leaves_per_swap as usize;

        let balance: u64 = input_leave_amounts.iter().sum();
        let optimal_leaves = self.swap_minimizing_leaves(balance);

        let wallet_counter = Self::count_occurrences(input_leave_amounts);
        let optimal_counter = Self::count_occurrences(&optimal_leaves);

        let leaves_to_give = Self::subtract_counters(&wallet_counter, &optimal_counter);
        let leaves_to_receive = Self::subtract_counters(&optimal_counter, &wallet_counter);

        let mut give = Self::counter_to_flat_array(&leaves_to_give);
        let mut receive = Self::counter_to_flat_array(&leaves_to_receive);

        // Build swaps by balancing give/receive batches
        let mut swaps = Vec::new();
        let mut to_give_batch: Vec<u64> = Vec::new();
        let mut to_receive_batch: Vec<u64> = Vec::new();

        while !give.is_empty() || !receive.is_empty() {
            let give_sum: u64 = to_give_batch.iter().sum();
            let receive_sum: u64 = to_receive_batch.iter().sum();

            if give_sum > receive_sum {
                if receive.is_empty() {
                    break;
                }
                to_receive_batch.push(receive.remove(0));
            } else {
                if give.is_empty() {
                    break;
                }
                to_give_batch.push(give.remove(0));
            }

            let give_sum: u64 = to_give_batch.iter().sum();
            let receive_sum: u64 = to_receive_batch.iter().sum();

            if !to_give_batch.is_empty() && !to_receive_batch.is_empty() && give_sum == receive_sum
            {
                // Create swap, potentially splitting if too large
                if to_give_batch.len() > max_leaves {
                    // Split give batch into chunks
                    for chunk in to_give_batch.chunks(max_leaves) {
                        let chunk_sum: u64 = chunk.iter().sum();
                        swaps.push(OptimizationSwap {
                            leaves_to_give: chunk.to_vec(),
                            leaves_to_receive: Self::greedy_leaves(chunk_sum),
                        });
                    }
                } else if to_receive_batch.len() > max_leaves {
                    // Find a valid cutoff for receive batch
                    for cutoff in (1..=max_leaves).rev() {
                        let sum_cut: u64 = to_receive_batch.iter().take(cutoff).sum();
                        let remainder = give_sum - sum_cut;
                        let mut alternate_batch: Vec<u64> =
                            to_receive_batch.iter().take(cutoff).copied().collect();
                        alternate_batch.extend(Self::greedy_leaves(remainder));

                        if alternate_batch.len() <= max_leaves {
                            swaps.push(OptimizationSwap {
                                leaves_to_give: to_give_batch.clone(),
                                leaves_to_receive: alternate_batch,
                            });
                            break;
                        }
                    }
                } else {
                    swaps.push(OptimizationSwap {
                        leaves_to_give: to_give_batch.clone(),
                        leaves_to_receive: to_receive_batch.clone(),
                    });
                }

                to_give_batch.clear();
                to_receive_batch.clear();
            }
        }

        swaps
    }

    /// Generates the optimal leaf values for a given balance that minimize transfer swaps.
    ///
    /// For each power-of-2 denomination (starting from smallest), tries to include it
    /// up to `multiplicity` times. Any remainder is handled by greedy decomposition.
    fn swap_minimizing_leaves(&self, amount: u64) -> Vec<u64> {
        let multiplicity = self.config.multiplicity;
        let mut result = Vec::new();
        let mut remaining = amount;

        // Iterate through powers of 2 from smallest to largest
        let mut power = 1u64;
        while power <= amount {
            for _ in 0..multiplicity {
                if remaining >= power {
                    remaining -= power;
                    result.push(power);
                }
            }
            // Prevent overflow
            if power > u64::MAX / 2 {
                break;
            }
            power *= 2;
        }

        // Handle any remaining balance with greedy decomposition
        result.extend(Self::greedy_leaves(remaining));

        result.sort();
        result
    }

    /// Greedy algorithm to break down a value into power-of-2 denominations.
    /// Returns values sorted in ascending order.
    fn greedy_leaves(mut value: u64) -> Vec<u64> {
        let mut result = Vec::new();
        let mut power = 1u64 << 63; // Start from highest power of 2

        while value > 0 && power > 0 {
            while value >= power {
                result.push(power);
                value -= power;
            }
            power /= 2;
        }

        result.sort();
        result
    }

    fn count_occurrences(values: &[u64]) -> std::collections::HashMap<u64, u64> {
        let mut counter = std::collections::HashMap::new();
        for &v in values {
            *counter.entry(v).or_insert(0) += 1;
        }
        counter
    }

    fn subtract_counters(
        a: &std::collections::HashMap<u64, u64>,
        b: &std::collections::HashMap<u64, u64>,
    ) -> std::collections::HashMap<u64, u64> {
        let mut result = std::collections::HashMap::new();
        for (&k, &v) in a {
            let b_count = b.get(&k).copied().unwrap_or(0);
            if v > b_count {
                result.insert(k, v - b_count);
            }
        }
        result
    }

    /// Converts a counter map to a flat array, sorted by key ascending.
    fn counter_to_flat_array(counter: &std::collections::HashMap<u64, u64>) -> Vec<u64> {
        let mut result = Vec::new();
        let mut keys: Vec<_> = counter.keys().collect();
        keys.sort(); // Sort ascending (matching TS reference)

        for &k in keys {
            let count = counter[&k];
            for _ in 0..count {
                result.push(k);
            }
        }
        result
    }

    fn emit_event(&self, event: OptimizationEvent) {
        if let Some(handler) = &self.event_handler {
            handler.on_optimization_event(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimization_options_validation() {
        let valid = OptimizationOptions {
            auto_enabled: true,
            multiplicity: 2,
            max_leaves_per_swap: 64,
        };
        assert!(valid.validate().is_ok());

        // multiplicity 0 is valid
        let multiplicity_zero = OptimizationOptions {
            multiplicity: 0,
            ..valid.clone()
        };
        assert!(multiplicity_zero.validate().is_ok());

        let invalid_multiplicity_high = OptimizationOptions {
            multiplicity: 6,
            ..valid.clone()
        };
        assert!(invalid_multiplicity_high.validate().is_err());

        let invalid_max_leaves = OptimizationOptions {
            max_leaves_per_swap: 0,
            ..valid
        };
        assert!(invalid_max_leaves.validate().is_err());
    }

    #[test]
    fn test_greedy_leaves() {
        let leaves = LeafOptimizer::greedy_leaves(100);
        assert_eq!(leaves.iter().sum::<u64>(), 100);

        let leaves = LeafOptimizer::greedy_leaves(255);
        assert_eq!(leaves.iter().sum::<u64>(), 255);
        // 255 = 128 + 64 + 32 + 16 + 8 + 4 + 2 + 1
        assert_eq!(leaves.len(), 8);
    }

    #[test]
    fn test_count_occurrences() {
        let values = vec![100, 200, 100, 300, 100];
        let counter = LeafOptimizer::count_occurrences(&values);
        assert_eq!(counter.get(&100), Some(&3));
        assert_eq!(counter.get(&200), Some(&1));
        assert_eq!(counter.get(&300), Some(&1));
    }

    #[test]
    fn test_subtract_counters() {
        let mut a = std::collections::HashMap::new();
        a.insert(100, 5);
        a.insert(200, 3);

        let mut b = std::collections::HashMap::new();
        b.insert(100, 2);
        b.insert(200, 3);
        b.insert(300, 1);

        let result = LeafOptimizer::subtract_counters(&a, &b);
        assert_eq!(result.get(&100), Some(&3));
        assert_eq!(result.get(&200), None); // Equal, so not in result
        assert_eq!(result.get(&300), None); // Not in a
    }
}
