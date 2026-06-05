use std::sync::{Arc, Mutex};

use platform_utils::tokio;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tracing::{debug, error, info, trace, warn};

use crate::{
    services::Swap,
    tree::{
        ReservationPurpose, SelectLeavesOptions, TargetAmounts, TreeNode, TreeService,
        TreeServiceError,
    },
};

const MAX_PLANNING_ITERATIONS: u32 = 8;

/// Default maximum number of leaves per swap round
pub const DEFAULT_MAX_LEAVES_PER_SWAP: u32 = 64;

/// Configuration options for leaf optimization.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LeafOptimizationOptions {
    /// Controls the optimization aggressiveness. Minimum value is 0, maximum value is 5.
    /// Higher values create more leaves for flexibility but may slow down operations.
    pub multiplicity: u8,
    /// Soft limit on the number of leaves per swap round.
    pub max_leaves_per_swap: u32,
}

impl Default for LeafOptimizationOptions {
    fn default() -> Self {
        Self {
            multiplicity: 1,
            max_leaves_per_swap: DEFAULT_MAX_LEAVES_PER_SWAP,
        }
    }
}

impl LeafOptimizationOptions {
    pub fn validate(&self) -> Result<(), TreeServiceError> {
        if self.multiplicity > 5 {
            warn!(
                "Multiplicity is greater than 5, you should only use this for high concurrency scenarios"
            );
        }
        if self.max_leaves_per_swap == 0 {
            return Err(TreeServiceError::Generic(
                "max_leaves_per_swap must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// Internal flag tracking whether an optimization run is in flight.
///
/// Held in an `Arc<Mutex<_>>` so the public `is_running()` getter, the
/// atomic check-and-set in `start`/`run`, and the cleanup in
/// [`RunningGuard`]'s `Drop` all coordinate through the same lock.
#[derive(Clone, Debug, Default)]
struct RunState {
    is_running: bool,
}

/// Events emitted during the lifecycle of a background ("auto")
/// optimization run.
///
/// Only emitted by runs spawned via [`LeafOptimizer::start`]. Synchronous
/// runs driven by [`LeafOptimizer::run`] do not fire events — their result
/// is returned via [`OptimizationOutcome`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum AutoOptimizationEvent {
    /// Optimization has started with the given number of rounds.
    Started { total_rounds: u32 },
    /// A round has completed.
    RoundCompleted {
        current_round: u32,
        total_rounds: u32,
    },
    /// Optimization completed successfully.
    Completed,
    /// Optimization was cancelled.
    Cancelled,
    /// Optimization failed with an error.
    Failed { error: String },
    /// Optimization was skipped because leaves are already optimal.
    Skipped,
}

/// Handler invoked for each [`AutoOptimizationEvent`] emitted by a
/// background optimization run.
pub trait AutoOptimizationEventHandler: Send + Sync {
    fn on_auto_optimization_event(&self, event: AutoOptimizationEvent);
}

/// Outcome of a [`LeafOptimizer::run`] invocation.
///
/// `rounds_executed` always refers to rounds executed *by this call*. The
/// optimizer holds no cross-call state — callers driving a capped loop
/// maintain their own cumulative counter if they want one.
///
/// A `Completed { rounds_executed: 0 }` outcome means the wallet was
/// already optimal at call time (no swap was needed).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum OptimizationOutcome {
    /// All planned optimization work was executed. Returned by uncapped
    /// runs on success, and by capped runs that executed what the planner
    /// classified as the final round. `rounds_executed == 0` means the
    /// wallet was already optimal — no work was performed.
    Completed { rounds_executed: u32 },
    /// A round ran but more rounds remain. Only emitted by capped runs;
    /// the caller should invoke [`LeafOptimizer::run`] again to continue.
    InProgress,
}

/// Errors returned by [`LeafOptimizer::run`].
#[derive(Debug, thiserror::Error)]
pub enum OptimizationError {
    /// Another optimization run (auto or manual) is in progress. The caller
    /// can retry later.
    #[error("Optimization is already in progress")]
    AlreadyRunning,
    /// The run was cancelled via [`LeafOptimizer::cancel`].
    #[error("Optimization was cancelled")]
    Cancelled,
    /// Underlying tree-service error.
    #[error(transparent)]
    Tree(#[from] TreeServiceError),
}

/// Represents a single swap operation plan for the optimization process.
#[derive(Clone, Debug, PartialEq)]
struct SwapPlan {
    /// The leaf values to give up in this swap.
    pub leaves_to_give: Vec<u64>,
    /// The leaf values to receive in this swap.
    pub leaves_to_receive: Vec<u64>,
}

/// A computed optimization plan plus a convergence hint from the planner.
#[derive(Clone, Debug, PartialEq)]
struct OptimizationPlan {
    swaps: Vec<SwapPlan>,
    /// `true` only when executing every swap in `swaps` is guaranteed to
    /// leave the wallet fully optimal (no further planning iterations
    /// needed). `false` is conservative — the planner sets it whenever it
    /// had to split batches or fall back to greedy fillers, since those
    /// emit swaps whose results are not the global optimum.
    fully_converges: bool,
}

/// RAII guard that automatically clears the running state when dropped.
/// Used to ensure the running state is always cleared even if the optimization fails.
#[derive(Clone)]
struct RunningGuard {
    state: Arc<Mutex<RunState>>,
    terminated: Arc<Notify>,
}

impl RunningGuard {
    fn new(state: Arc<Mutex<RunState>>, terminated: Arc<Notify>) -> Self {
        Self { state, terminated }
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.is_running = false;
        drop(state);

        self.terminated.notify_waiters();
    }
}
/// Service responsible for optimizing leaf denominations.
///
/// The optimizer transforms the current set of leaves into an optimal set
/// that minimizes the probability of needing swaps during transfers or
/// maximizes the amount that can be unilaterally exited (depending on the configuration).
/// It operates in multiple rounds, each performing a swap operation.
pub struct LeafOptimizer {
    config: LeafOptimizationOptions,
    swap_service: Arc<Swap>,
    tree_service: Arc<dyn TreeService>,
    state: Arc<Mutex<RunState>>,
    cancel_tx: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
    terminated: Arc<Notify>,
    event_handler: Option<Arc<dyn AutoOptimizationEventHandler>>,
}

impl LeafOptimizer {
    pub fn new(
        config: LeafOptimizationOptions,
        swap_service: Arc<Swap>,
        tree_service: Arc<dyn TreeService>,
        event_handler: Option<Arc<dyn AutoOptimizationEventHandler>>,
    ) -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        Self {
            config,
            swap_service,
            tree_service,
            state: Arc::new(Mutex::new(RunState::default())),
            cancel_tx,
            cancel_rx,
            terminated: Arc::new(Notify::new()),
            event_handler,
        }
    }

    /// Checks if optimization is currently running and may have leaves in use.
    /// Used to determine if optimization should be cancelled when payments fail.
    pub fn is_running(&self) -> bool {
        self.state.lock().unwrap().is_running
    }

    fn should_optimize(&self, leaves: &[TreeNode]) -> bool {
        if self.config.multiplicity == 0 {
            let leave_amounts = leaves.iter().map(|leaf| leaf.value).collect::<Vec<u64>>();

            let plan = maximize_unilateral_exit(&leave_amounts, self.config.max_leaves_per_swap);

            let num_inputs: usize = plan.swaps.iter().map(|s| s.leaves_to_give.len()).sum();
            let num_outputs: usize = plan.swaps.iter().map(|s| s.leaves_to_receive.len()).sum();

            num_inputs > num_outputs
        } else {
            true
        }
    }

    /// Starts the auto-optimizer in the background.
    ///
    /// Spawns the optimization work in a background task and returns
    /// immediately. Progress is reported via [`AutoOptimizationEvent`]s
    /// delivered to the configured handler.
    ///
    /// Returns early (without spawning) if optimization is already running.
    pub async fn start(self: &Arc<Self>) {
        let running_guard = {
            let mut state = self.state.lock().unwrap();
            if state.is_running {
                trace!("Optimization already running, skipping");
                return;
            }
            state.is_running = true;
            // Reset the cancel signal under the same lock that gates
            // `is_running`. A concurrent `cancel()` call cannot interleave
            // between these two writes, so an arriving cancel signal can't
            // be clobbered by a stale reset.
            let _ = self.cancel_tx.send(false);
            RunningGuard::new(Arc::clone(&self.state), Arc::clone(&self.terminated))
        };

        let optimizer = Arc::clone(self);
        tokio::spawn(async move {
            // The auto path always runs to completion and emits events.
            // Errors are logged; outcomes are reported via events.
            let _ = optimizer.run_inner(None, true, running_guard).await;
        });
    }

    /// Manually drives leaf optimization.
    ///
    /// - `max_rounds = None`: run until no further optimization is productive.
    /// - `max_rounds = Some(n)`: execute up to `n` rounds and return so the
    ///   caller can drive progress externally.
    ///
    /// Returns:
    /// - [`OptimizationOutcome::Completed`] if all planned work was done
    ///   (uncapped run, or capped run after the final round). A
    ///   `rounds_executed` of `0` means no work was needed.
    /// - [`OptimizationOutcome::InProgress`] if a round ran but more remain.
    ///
    /// Errors:
    /// - [`OptimizationError::AlreadyRunning`] if another run (auto or
    ///   manual) is in flight.
    /// - [`OptimizationError::Cancelled`] if [`Self::cancel`] was invoked
    ///   while the run was in progress.
    ///
    /// Unlike [`Self::start`], this path does not emit
    /// [`AutoOptimizationEvent`]s — only the auto path produces events.
    pub async fn run(
        &self,
        max_rounds: Option<u32>,
    ) -> Result<OptimizationOutcome, OptimizationError> {
        // Atomically claim the running slot. If someone (auto or another
        // manual call) already has it, reject. The cancel reset happens
        // under the same lock so a concurrent `cancel()` can't be
        // clobbered (see `start` for the same pattern).
        let running_guard = {
            let mut state = self.state.lock().unwrap();
            if state.is_running {
                return Err(OptimizationError::AlreadyRunning);
            }
            state.is_running = true;
            let _ = self.cancel_tx.send(false);
            RunningGuard::new(Arc::clone(&self.state), Arc::clone(&self.terminated))
        };

        self.run_inner(max_rounds, false, running_guard).await
    }

    /// Shared driver for both the auto path (via [`Self::start`]) and the
    /// manual path (via [`Self::run`]).
    ///
    /// `emit_events = true` mirrors the legacy auto behaviour:
    /// `Started`/`RoundCompleted`/terminal events are emitted to the
    /// handler. `emit_events = false` runs silently and reports via the
    /// returned [`OptimizationOutcome`].
    ///
    /// Terminal detection for `max_rounds`-capped runs combines the plan's
    /// length with the planner's convergence hint: a single-swap plan
    /// whose `fully_converges` flag is `true` means executing that swap
    /// leaves the wallet fully optimal, so we return `Completed` in the
    /// same call. Otherwise (multi-swap plan, or single-swap plan with
    /// `fully_converges = false` from the give-too-big / receive-too-big
    /// split branches), we return `InProgress` and let the caller's next
    /// call discover the terminal state.
    async fn run_inner(
        &self,
        max_rounds: Option<u32>,
        emit_events: bool,
        _running_guard: RunningGuard,
    ) -> Result<OptimizationOutcome, OptimizationError> {
        // The cancel signal was reset by `run`/`start` under the same
        // lock that claimed `is_running`, so no reset is needed here.

        let mut rounds_this_call: u32 = 0;
        let mut started_emitted = false;

        for iteration in 1..=MAX_PLANNING_ITERATIONS {
            if *self.cancel_rx.borrow() {
                debug!("Optimization cancelled before iteration {iteration}");
                if emit_events {
                    self.emit_event(AutoOptimizationEvent::Cancelled);
                }
                return Err(OptimizationError::Cancelled);
            }

            let leaves = self.tree_service.list_leaves().await?.available;
            if leaves.is_empty() {
                break;
            }

            // `should_optimize` is a quick pre-check on the very first
            // pass when no work has happened yet; later iterations have
            // already done real swaps, so we trust the planner.
            if iteration == 1 && rounds_this_call == 0 && !self.should_optimize(&leaves) {
                debug!("Optimization not needed, skipping");
                break;
            }

            let plan = calculate_optimization_swaps(
                &leaves.iter().map(|l| l.value).collect::<Vec<u64>>(),
                self.config.multiplicity,
                self.config.max_leaves_per_swap,
            );
            let swaps = plan.swaps;
            let plan_fully_converges = plan.fully_converges;

            if swaps.is_empty() {
                break;
            }

            let rounds_in_iter = swaps.len() as u32;
            // A single-swap, fully-converging plan means executing this
            // swap leaves the wallet optimal — capped runs use this to
            // return `Completed` in the same call.
            let plan_is_terminal = rounds_in_iter == 1 && plan_fully_converges;
            let last_total_estimate = rounds_this_call + rounds_in_iter;

            info!(
                "Optimization iteration {iteration}: {} rounds, {} input leaves, {} output leaves",
                rounds_in_iter,
                swaps.iter().map(|s| s.leaves_to_give.len()).sum::<usize>(),
                swaps
                    .iter()
                    .map(|s| s.leaves_to_receive.len())
                    .sum::<usize>()
            );

            if emit_events && !started_emitted {
                self.emit_event(AutoOptimizationEvent::Started {
                    total_rounds: last_total_estimate,
                });
                started_emitted = true;
            }

            for swap in swaps {
                if *self.cancel_rx.borrow() {
                    debug!("Optimization cancelled mid-iteration");
                    if emit_events {
                        self.emit_event(AutoOptimizationEvent::Cancelled);
                    }
                    return Err(OptimizationError::Cancelled);
                }

                let round_number = rounds_this_call + 1;
                match self
                    .execute_one_swap(swap, round_number, last_total_estimate, emit_events)
                    .await
                {
                    Ok(true) => {
                        rounds_this_call += 1;
                    }
                    Ok(false) => {
                        // Round skipped (insufficient funds / busy);
                        // try the next planned swap.
                        continue;
                    }
                    Err(e) => {
                        if emit_events {
                            self.emit_event(AutoOptimizationEvent::Failed {
                                error: e.to_string(),
                            });
                        }
                        return Err(OptimizationError::Tree(e));
                    }
                }

                // Capped runs return after reaching `max_rounds`. A
                // single-swap, fully-converging plan is terminal — return
                // `Completed` in this call. Otherwise return `InProgress`
                // and let the caller drive the loop.
                if let Some(max) = max_rounds
                    && rounds_this_call >= max
                {
                    return Ok(if plan_is_terminal {
                        OptimizationOutcome::Completed {
                            rounds_executed: rounds_this_call,
                        }
                    } else {
                        OptimizationOutcome::InProgress
                    });
                }
            }
        }

        if rounds_this_call == 0 {
            if emit_events {
                self.emit_event(AutoOptimizationEvent::Skipped);
            }
        } else {
            info!("Leaf optimization completed successfully ({rounds_this_call} rounds executed)");
            if emit_events {
                self.emit_event(AutoOptimizationEvent::Completed);
            }
        }
        Ok(OptimizationOutcome::Completed {
            rounds_executed: rounds_this_call,
        })
    }

    /// Cancels the ongoing optimization and waits for it to fully stop.
    ///
    /// Sets a cancellation flag that is checked between rounds; the current
    /// round will complete before stopping. Blocks until the run has fully
    /// stopped and leaves reserved for swaps are released.
    pub async fn cancel(&self) -> Result<(), TreeServiceError> {
        if !self.state.lock().unwrap().is_running {
            debug!("No optimization running to cancel");
            return Ok(());
        }

        info!("Cancelling leaf optimization and waiting for completion");

        // Create the notified future BEFORE sending cancel signal to avoid race conditions
        let notified = self.terminated.notified();

        let _ = self.cancel_tx.send(true);

        // Double-check: if optimization already stopped between our first check
        // and creating the notified future, we can return early
        if !self.state.lock().unwrap().is_running {
            debug!("Optimization already stopped");
            return Ok(());
        }

        notified.await;

        debug!("Optimization cancelled and stopped");
        Ok(())
    }

    /// Executes a single planned swap. Returns `Ok(true)` if the swap was
    /// executed, `Ok(false)` if it was skipped (insufficient funds, busy),
    /// or `Err` on swap failure.
    async fn execute_one_swap(
        &self,
        swap: SwapPlan,
        round: u32,
        total_rounds: u32,
        emit_events: bool,
    ) -> Result<bool, TreeServiceError> {
        let swap_reservation = match self
            .tree_service
            .select_leaves(
                Some(&TargetAmounts::new_exact_denominations(
                    swap.leaves_to_give.clone(),
                )),
                ReservationPurpose::Swap,
                SelectLeavesOptions::no_wait(),
            )
            .await
        {
            Ok(reservation) => reservation,
            Err(TreeServiceError::InsufficientFunds) => {
                debug!("Optimization round {} skipped: insufficient funds", round);
                return Ok(false);
            }
            Err(e) => {
                debug!("Optimization round {} skipped due to error: {:?}", round, e);
                return Ok(false);
            }
        };

        debug!(
            "Executing optimization round {round}/{total_rounds}: give {} leaves {:?} ({} sats), receive {} leaves {:?} ({} sats)",
            swap.leaves_to_give.len(),
            swap.leaves_to_give,
            swap.leaves_to_give.iter().sum::<u64>(),
            swap.leaves_to_receive.len(),
            swap.leaves_to_receive,
            swap.leaves_to_receive.iter().sum::<u64>(),
        );

        match self
            .swap_service
            .swap_leaves(&swap_reservation.leaves, Some(swap.leaves_to_receive))
            .await
        {
            Ok(new_leaves) => {
                let gave_values: Vec<u64> =
                    swap_reservation.leaves.iter().map(|l| l.value).collect();
                let received_values: Vec<u64> = new_leaves.iter().map(|l| l.value).collect();

                if let Err(e) = self
                    .tree_service
                    .finalize_reservation(swap_reservation.id, Some(&new_leaves))
                    .await
                {
                    error!(
                        "Failed to finalize optimization reservation, proceeding with optimization. {:?}",
                        e
                    );
                }

                if emit_events {
                    self.emit_event(AutoOptimizationEvent::RoundCompleted {
                        current_round: round,
                        total_rounds,
                    });
                }

                debug!(
                    "Completed optimization round {}/{}: gave {} leaves {:?} ({} sats), received {} leaves {:?} ({} sats)",
                    round,
                    total_rounds,
                    gave_values.len(),
                    gave_values,
                    gave_values.iter().sum::<u64>(),
                    received_values.len(),
                    received_values,
                    received_values.iter().sum::<u64>(),
                );
                Ok(true)
            }
            Err(e) => {
                let reserved_leaf_ids: Vec<String> = swap_reservation
                    .leaves
                    .iter()
                    .map(|l| l.id.to_string())
                    .collect();
                warn!(
                    "leaf_lifecycle swap_failed_in_optimize: reservation={} round={} leaf_ids={:?} error={:?}",
                    swap_reservation.id, round, reserved_leaf_ids, e
                );
                if let Err(cancel_err) =
                    self.tree_service.cancel_reservation(swap_reservation).await
                {
                    error!(
                        "Failed to cancel reservation on optimization round failure: {cancel_err:?}"
                    );
                }

                Err(TreeServiceError::Generic(format!(
                    "Failed to perform swap in optimization round {round}: {e:?}"
                )))
            }
        }
    }

    fn emit_event(&self, event: AutoOptimizationEvent) {
        if let Some(handler) = &self.event_handler {
            handler.on_auto_optimization_event(event);
        }
    }
}

fn calculate_optimization_swaps(
    input_leave_amounts: &[u64],
    multiplicity: u8,
    max_leaves_per_swap: u32,
) -> OptimizationPlan {
    if multiplicity == 0 {
        maximize_unilateral_exit(input_leave_amounts, max_leaves_per_swap)
    } else {
        minimize_transfer_swap(input_leave_amounts, multiplicity, max_leaves_per_swap)
    }
}

/// Calculates the swaps needed to optimize when maximizing unilateral exit.
///
/// Generates swaps that will result in the unilateral exit maximizing set of leaves.
///
/// Convergence: when all wallet leaves fit in a single batch
/// (`wallet.len() <= max_leaves_per_swap`), the lone remainder batch is
/// swapped to `greedy_leaves(total)` — the global optimum — so the plan
/// fully converges. When the wallet spans multiple batches, each batch is
/// independently swapped to `greedy_leaves(batch_sum)`, and the union of
/// per-batch greedies is generally not equal to `greedy_leaves(total)`,
/// so subsequent planning iterations may find more work.
fn maximize_unilateral_exit(
    input_leave_amounts: &[u64],
    max_leaves_per_swap: u32,
) -> OptimizationPlan {
    let max_leaves_per_swap_usize = max_leaves_per_swap as usize;
    let mut swaps = Vec::new();
    let mut batch: Vec<u64> = Vec::new();

    // Sort leaves ascending
    let mut leaves: Vec<u64> = input_leave_amounts.to_vec();
    leaves.sort();
    let fully_converges = leaves.len() <= max_leaves_per_swap_usize;

    // Process leaves in batches of up to approximately max_leaves_per_swap
    while !leaves.is_empty() {
        batch.push(leaves.remove(0));
        let batch_sum: u64 = batch.iter().sum();
        let target = greedy_leaves(batch_sum);

        if batch.len() >= max_leaves_per_swap_usize || target.len() >= max_leaves_per_swap_usize {
            if target != batch {
                swaps.push(SwapPlan {
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
        let target = greedy_leaves(batch_sum);

        if target != batch {
            swaps.push(SwapPlan {
                leaves_to_give: batch,
                leaves_to_receive: target,
            });
        }
    }

    OptimizationPlan {
        swaps,
        fully_converges,
    }
}

/// Calculates the swaps needed to optimize when minimizing transfer swaps.
///
/// Generates swaps that will minimize the probability of needing to swap during a transfer.
///
/// Convergence: when neither the give nor receive batch needs splitting,
/// the plan converges (each emitted swap has the exact `optimal_leaves`
/// shape on the receive side). When either batch is split into smaller
/// chunks, the chunks fall back to `greedy_leaves(chunk_sum)` on the
/// receive side, which is not the multiplicity=N optimal shape — so
/// subsequent planning iterations may find more work.
fn minimize_transfer_swap(
    input_leave_amounts: &[u64],
    multiplicity: u8,
    max_leaves_per_swap: u32,
) -> OptimizationPlan {
    let max_leaves = max_leaves_per_swap as usize;

    let balance: u64 = input_leave_amounts.iter().sum();
    let optimal_leaves = swap_minimizing_leaves(balance, multiplicity);

    let wallet_counter = count_occurrences(input_leave_amounts);
    let optimal_counter = count_occurrences(&optimal_leaves);

    let leaves_to_give = subtract_counters(&wallet_counter, &optimal_counter);
    let leaves_to_receive = subtract_counters(&optimal_counter, &wallet_counter);

    let mut give = counter_to_flat_array(&leaves_to_give);
    let mut receive = counter_to_flat_array(&leaves_to_receive);

    // Sanity check: give and receive sums should match
    if give.iter().sum::<u64>() != receive.iter().sum::<u64>() {
        error!(
            "Unexpected: Give and receive sums do not match. Give: {give:?}, Receive: {receive:?}"
        );
        return OptimizationPlan {
            swaps: vec![],
            fully_converges: false,
        };
    }

    // Build swaps by balancing give/receive batches
    let mut swaps = Vec::new();
    let mut to_give_batch: Vec<u64> = Vec::new();
    let mut to_receive_batch: Vec<u64> = Vec::new();
    let mut fully_converges = true;

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

        if !to_give_batch.is_empty() && !to_receive_batch.is_empty() && give_sum == receive_sum {
            // Create swap, potentially splitting if too large
            if to_give_batch.len() > max_leaves {
                // Split give batch into chunks
                // TODO: consider improving this fallback logic in order to minimize the deviation from the optimal set.
                fully_converges = false;
                for chunk in to_give_batch.chunks(max_leaves) {
                    let chunk_sum: u64 = chunk.iter().sum();
                    swaps.push(SwapPlan {
                        leaves_to_give: chunk.to_vec(),
                        leaves_to_receive: greedy_leaves(chunk_sum),
                    });
                }
            } else if to_receive_batch.len() > max_leaves {
                // Find a valid cutoff for receive batch
                fully_converges = false;
                let mut found_valid_cutoff = false;
                for cutoff in (1..=max_leaves).rev() {
                    let sum_cut: u64 = to_receive_batch.iter().take(cutoff).sum();
                    let remainder = give_sum - sum_cut;
                    let mut alternate_batch: Vec<u64> =
                        to_receive_batch.iter().take(cutoff).copied().collect();
                    alternate_batch.extend(greedy_leaves(remainder));

                    if alternate_batch.len() <= max_leaves {
                        swaps.push(SwapPlan {
                            leaves_to_give: to_give_batch.clone(),
                            leaves_to_receive: alternate_batch,
                        });
                        found_valid_cutoff = true;
                        break;
                    }
                }

                if !found_valid_cutoff {
                    error!(
                        "Unexpected: No valid cutoff found for receive batch of length {}, skipping swap.. Maybe max_leaves_per_swap is too low.",
                        to_receive_batch.len()
                    );
                }
            } else {
                swaps.push(SwapPlan {
                    leaves_to_give: to_give_batch.clone(),
                    leaves_to_receive: to_receive_batch.clone(),
                });
            }

            to_give_batch.clear();
            to_receive_batch.clear();
        }
    }

    OptimizationPlan {
        swaps,
        fully_converges,
    }
}

/// Generates the optimal leaf values for a given balance that minimize transfer swaps.
///
/// For each power-of-2 denomination (starting from smallest), tries to include it
/// up to `multiplicity` times. Any remainder is handled by greedy decomposition.
fn swap_minimizing_leaves(amount: u64, multiplicity: u8) -> Vec<u64> {
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
    result.extend(greedy_leaves(remaining));

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

#[cfg(test)]
mod tests {
    use super::*;
    use macros::{async_test_all, test_all};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_optimization_options_validation() {
        let valid = LeafOptimizationOptions {
            multiplicity: 2,
            max_leaves_per_swap: 64,
        };
        assert!(valid.validate().is_ok());

        // multiplicity 0 is valid
        let multiplicity_zero = LeafOptimizationOptions {
            multiplicity: 0,
            ..valid.clone()
        };
        assert!(multiplicity_zero.validate().is_ok());

        let invalid_max_leaves = LeafOptimizationOptions {
            max_leaves_per_swap: 0,
            ..valid
        };
        assert!(invalid_max_leaves.validate().is_err());
    }

    #[test_all]
    fn test_calculate_optimization_swaps() {
        // Test optimize for unilateral exit (multiplicity = 0). Wallet
        // fits in one batch → fully converges.
        assert_eq!(
            calculate_optimization_swaps(&[8], 0, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![],
                fully_converges: true,
            }
        );
        assert_eq!(
            calculate_optimization_swaps(&[16], 0, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![],
                fully_converges: true,
            }
        );
        assert_eq!(
            calculate_optimization_swaps(
                &[16, 16, 16, 16, 16, 16, 16, 16],
                0,
                DEFAULT_MAX_LEAVES_PER_SWAP
            ),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![16, 16, 16, 16, 16, 16, 16, 16],
                    leaves_to_receive: vec![128],
                }],
                fully_converges: true,
            }
        );
        assert_eq!(
            calculate_optimization_swaps(&[100000], 0, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![100000],
                    leaves_to_receive: vec![32, 128, 512, 1024, 32768, 65536],
                }],
                fully_converges: true,
            }
        );

        // Test optimize for swap minimization (multiplicity = 1). All
        // wallets here fit in one swap (no split branch) → converges.
        assert_eq!(
            calculate_optimization_swaps(&[8], 1, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![8],
                    leaves_to_receive: vec![1, 1, 2, 4],
                }],
                fully_converges: true,
            }
        );
        assert_eq!(
            calculate_optimization_swaps(&[1, 4], 1, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![4],
                    leaves_to_receive: vec![2, 2],
                }],
                fully_converges: true,
            }
        );
        assert_eq!(
            calculate_optimization_swaps(&[1, 16], 1, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![16],
                    leaves_to_receive: vec![2, 2, 4, 8],
                }],
                fully_converges: true,
            }
        );
    }

    #[test_all]
    fn test_calculate_optimization_swaps_does_not_converge_when_split() {
        // multiplicity=0 wallet with more leaves than fits one batch:
        // multiple batches → no convergence guarantee from one iteration.
        let plan = calculate_optimization_swaps(&[16, 16, 16, 16], 0, 2);
        assert!(
            !plan.swaps.is_empty(),
            "expected at least one swap for non-greedy input"
        );
        assert!(
            !plan.fully_converges,
            "multi-batch unilateral-exit plans cannot guarantee convergence in one iteration"
        );

        // multiplicity=15 with a large balance forces the planner into
        // its receive-too-big branch (optimal set far exceeds
        // max_leaves_per_swap), so the emitted plan should NOT claim
        // convergence.
        let plan = calculate_optimization_swaps(&[50_000], 15, DEFAULT_MAX_LEAVES_PER_SWAP);
        assert!(
            !plan.swaps.is_empty(),
            "expected at least one swap when wallet differs from optimal"
        );
        assert!(
            !plan.fully_converges,
            "receive-too-big fallback in minimize_transfer_swap must not claim convergence"
        );
    }

    #[test_all]
    fn test_swap_minimizing_leaves() {
        assert_eq!(swap_minimizing_leaves(0, 1), Vec::<u64>::new());
        assert_eq!(swap_minimizing_leaves(1, 1), vec![1]);
        assert_eq!(
            swap_minimizing_leaves(100, 1),
            vec![1, 1, 2, 4, 4, 8, 16, 32, 32]
        );
        assert_eq!(
            swap_minimizing_leaves(255, 1),
            vec![1, 2, 4, 8, 16, 32, 64, 128]
        );
        assert_eq!(
            swap_minimizing_leaves(256, 1),
            vec![1, 1, 2, 4, 8, 16, 32, 64, 128]
        );
    }

    #[test_all]
    fn test_maximize_unilateral_exit() {
        assert_eq!(
            maximize_unilateral_exit(&[100, 64, 28, 1, 1], DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![1, 1, 28, 64, 100],
                    leaves_to_receive: vec![2, 64, 128]
                }],
                fully_converges: true,
            }
        );
        assert_eq!(
            maximize_unilateral_exit(&[1, 1, 1, 1, 1, 1, 1, 1], 2),
            OptimizationPlan {
                swaps: vec![
                    SwapPlan {
                        leaves_to_give: vec![1, 1],
                        leaves_to_receive: vec![2]
                    },
                    SwapPlan {
                        leaves_to_give: vec![1, 1],
                        leaves_to_receive: vec![2]
                    },
                    SwapPlan {
                        leaves_to_give: vec![1, 1],
                        leaves_to_receive: vec![2]
                    },
                    SwapPlan {
                        leaves_to_give: vec![1, 1],
                        leaves_to_receive: vec![2]
                    }
                ],
                // 8 leaves with max=2 = multiple batches: no single-iteration
                // convergence guarantee.
                fully_converges: false,
            }
        );
    }

    #[test_all]
    fn test_minimize_transfer_swap() {
        assert_eq!(
            minimize_transfer_swap(&[8], 1, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![8],
                    leaves_to_receive: vec![1, 1, 2, 4]
                }],
                fully_converges: true,
            }
        );
        assert_eq!(
            minimize_transfer_swap(&[100], 1, DEFAULT_MAX_LEAVES_PER_SWAP),
            OptimizationPlan {
                swaps: vec![SwapPlan {
                    leaves_to_give: vec![100],
                    leaves_to_receive: vec![1, 1, 2, 4, 4, 8, 16, 32, 32]
                }],
                fully_converges: true,
            }
        );
    }

    #[test_all]
    fn test_greedy_leaves() {
        assert_eq!(greedy_leaves(0), Vec::<u64>::new());
        assert_eq!(greedy_leaves(1), vec![1]);
        assert_eq!(greedy_leaves(100), vec![4, 32, 64]);
        assert_eq!(greedy_leaves(255), vec![1, 2, 4, 8, 16, 32, 64, 128]);
        assert_eq!(greedy_leaves(256), vec![256]);
    }

    #[test_all]
    fn test_count_occurrences() {
        let values = vec![100, 200, 100, 300, 100];
        let counter = count_occurrences(&values);
        assert_eq!(counter.get(&100), Some(&3));
        assert_eq!(counter.get(&200), Some(&1));
        assert_eq!(counter.get(&300), Some(&1));
    }

    #[test_all]
    fn test_subtract_counters() {
        let mut a = std::collections::HashMap::new();
        a.insert(100, 5);
        a.insert(200, 3);

        let mut b = std::collections::HashMap::new();
        b.insert(100, 2);
        b.insert(200, 3);
        b.insert(300, 1);

        let result = subtract_counters(&a, &b);
        assert_eq!(result.get(&100), Some(&3));
        assert_eq!(result.get(&200), None); // Equal, so not in result
        assert_eq!(result.get(&300), None); // Not in a
    }

    #[test_all]
    fn test_running_guard_drop_clears_state() {
        let state = Arc::new(std::sync::Mutex::new(RunState { is_running: true }));
        let terminated = Arc::new(tokio::sync::Notify::new());

        {
            let _guard = RunningGuard::new(Arc::clone(&state), Arc::clone(&terminated));
            assert!(state.lock().unwrap().is_running);
        } // guard dropped here

        // After drop, state should be cleared
        assert!(!state.lock().unwrap().is_running);
    }

    #[async_test_all]
    async fn test_running_guard_notifies_on_clear() {
        let state = Arc::new(std::sync::Mutex::new(RunState { is_running: true }));
        let terminated = Arc::new(tokio::sync::Notify::new());

        let notified = terminated.notified();

        let guard = RunningGuard::new(Arc::clone(&state), Arc::clone(&terminated));

        drop(guard);

        // Should be notified
        notified.await;
    }

    /// Calculate the maximum amount that can be unilaterally exited given fees per leaf
    fn calculate_max_unilateral_exit(leaves: &[u64], fee_per_leaf: u64) -> u64 {
        if leaves.is_empty() {
            return 0;
        }

        // Sort leaves in descending order (largest first) to maximize exit value per fee paid
        let mut sorted_leaves = leaves.to_vec();
        sorted_leaves.sort_by(|a, b| b.cmp(a));

        let mut total_exited = 0u64;

        for &leaf_value in &sorted_leaves {
            // Check if we can afford to exit this leaf
            let fee_cost = fee_per_leaf;
            if leaf_value > fee_cost {
                total_exited += leaf_value - fee_cost;
            } else {
                // If we can't afford to exit this leaf profitably, we're done
                // since smaller leaves will be even less profitable
                break;
            }
        }

        total_exited
    }

    /// Analysis of unilateral exit trade-offs across different multiplicities
    /// This analysis explores how multiplicity affects unilateral exit capability under various fee scenarios
    /// Run with: cargo test --package spark --lib leaf_optimizer::tests::unilateral_exit_trade_off_analysis -- --nocapture --ignored
    #[ignore]
    #[test_all]
    fn unilateral_exit_trade_off_analysis() {
        println!("=== Systematic Unilateral Exit Analysis ===");
        println!("Exploring multiplicity impact across fees and fund sizes\n");

        // Fees per leaf (sats)
        let fee_values = vec![5000, 10000, 20000, 40000];
        let total_funds_values = vec![10000, 100000, 1000000, 10000000];
        let multiplicities = vec![0, 1, 2, 3, 4, 5];

        for &fee_per_leaf in &fee_values {
            println!(
                "══════════════════════════════════════════════════════════════════════════════════════════════"
            );
            println!("FEE PER LEAF: {} SATS", fee_per_leaf);
            println!(
                "══════════════════════════════════════════════════════════════════════════════════════════════"
            );
            println!("Total Funds | Mult | Leaves | Exit Amount | Efficiency | Loss Amount");
            println!("------------|------|--------|-------------|------------|------------");

            for &total_funds in &total_funds_values {
                for &multiplicity in &multiplicities {
                    let leaves = if multiplicity == 0 {
                        greedy_leaves(total_funds)
                    } else {
                        swap_minimizing_leaves(total_funds, multiplicity)
                    };
                    let max_exit = calculate_max_unilateral_exit(&leaves, fee_per_leaf);
                    let efficiency = (max_exit as f64) / (total_funds as f64) * 100.0;
                    let loss = total_funds - max_exit;

                    println!(
                        "{:>10}k | {:>4} | {:>6} | {:>11} | {:>8.1}% | {:>10}",
                        total_funds / 1000,
                        multiplicity,
                        leaves.len(),
                        max_exit,
                        efficiency,
                        loss
                    );
                }
                println!("------------|------|--------|-------------|------------|------------");
            }

            println!();
        }
    }
}
