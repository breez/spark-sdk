//! Resolves the exit chains of stored leaves that lack one.
//!
//! A leaf is persisted as soon as the operation that produced it completes, without
//! its chain: resolving the chain costs an operator round trip, and an operation
//! that swaps often sends most of its new leaves straight back out, so the chains
//! of the ones that leave would be fetched and written for nothing. Fetching
//! afterwards pays that cost once, only for the leaves that stayed.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use platform_utils::time::SystemTime;
use platform_utils::tokio;
use platform_utils::tokio::sync::{Notify, watch};
use tracing::{trace, warn};

use crate::tree::{LeafPedigree, TreeNodeId, TreeService, TreeServiceError};

/// Delay before a leaf whose chain the operators did not complete is tried again.
/// Doubles per consecutive failure up to [`MAX_RETRY_DELAY`].
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30 * 60);

/// Wakes an [`ExitChainResolver`]. Cheap to clone and hold wherever leaves are
/// produced.
///
/// Signals coalesce: triggering repeatedly while a run is in progress schedules
/// exactly one more run, so a burst of operations does not queue a burst of
/// operator queries.
#[derive(Clone)]
pub struct ExitChainTrigger {
    notify: Arc<Notify>,
}

impl ExitChainTrigger {
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn trigger(&self) {
        self.notify.notify_one();
    }
}

impl Default for ExitChainTrigger {
    fn default() -> Self {
        Self::new()
    }
}

/// A chain is complete once it reaches a node with no parent. A leaf that is itself
/// a root needs no ancestors to be exitable.
fn is_complete(pedigree: &LeafPedigree) -> bool {
    match pedigree.ancestors.last() {
        Some(topmost) => topmost.parent_node_id.is_none(),
        None => pedigree.leaf.parent_node_id.is_none(),
    }
}

#[derive(Clone, Copy)]
struct Backoff {
    retry_at: SystemTime,
    delay: Duration,
}

pub struct ExitChainResolver {
    tree_service: Arc<dyn TreeService>,
    notify: Arc<Notify>,
    /// Leaves the operators would not complete, held off until their retry time.
    /// Deliberately in memory only: a restart is a good moment to try again.
    backoff: Mutex<HashMap<TreeNodeId, Backoff>>,
}

impl ExitChainResolver {
    pub fn new(tree_service: Arc<dyn TreeService>, trigger: &ExitChainTrigger) -> Self {
        Self {
            tree_service,
            notify: Arc::clone(&trigger.notify),
            backoff: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves what is missing once, without waiting to be triggered.
    ///
    /// Leaves the operators recently failed to complete stay backed off, so calling
    /// this in a loop does not turn into a retry storm.
    pub async fn resolve_missing(&self) -> Result<(), TreeServiceError> {
        self.resolve_missing_chains().await
    }

    /// Runs until cancelled, resolving missing chains on each trigger. The first
    /// pass happens without waiting, so chains missed while the wallet was down are
    /// picked up at startup.
    pub async fn run(&self, mut cancellation_token: watch::Receiver<()>) {
        loop {
            if let Err(e) = self.resolve_missing_chains().await {
                warn!("Failed to resolve exit chains: {e:?}");
            }
            tokio::select! {
                () = self.notify.notified() => {}
                _ = cancellation_token.changed() => {
                    trace!("Stopping exit chain resolver");
                    return;
                }
            }
        }
    }

    async fn resolve_missing_chains(&self) -> Result<(), TreeServiceError> {
        let missing = self.tree_service.leaves_missing_exit_chains().await?;
        self.retain_backoff(&missing.iter().cloned().collect());
        if missing.is_empty() {
            return Ok(());
        }

        let now = SystemTime::now();
        let candidates: Vec<TreeNodeId> = missing
            .into_iter()
            .filter(|leaf_id| self.is_due(leaf_id, now))
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        trace!("Resolving exit chains for {} leaves", candidates.len());

        let fetched = self
            .tree_service
            .fetch_pedigrees_from_operators(&candidates)
            .await;
        let (resolved, unresolved): (Vec<LeafPedigree>, Vec<LeafPedigree>) =
            fetched.into_iter().partition(is_complete);

        // A partial chain cannot back an exit on its own, but it is progress and
        // costs nothing to keep, so store whatever came back.
        let to_store: Vec<LeafPedigree> = resolved
            .iter()
            .cloned()
            .chain(
                unresolved
                    .iter()
                    .filter(|pedigree| !pedigree.ancestors.is_empty())
                    .cloned(),
            )
            .collect();
        if !to_store.is_empty() {
            self.tree_service.store_exit_chains(&to_store).await?;
        }

        self.record_outcomes(&resolved, &unresolved, now);
        Ok(())
    }

    fn is_due(&self, leaf_id: &TreeNodeId, now: SystemTime) -> bool {
        let backoff = self.backoff.lock().unwrap();
        backoff
            .get(leaf_id)
            .is_none_or(|entry| now >= entry.retry_at)
    }

    /// Clears the backoff of leaves that resolved and extends it for those that did
    /// not, so an unresolvable chain is not retried on every trigger.
    fn record_outcomes(
        &self,
        resolved: &[LeafPedigree],
        unresolved: &[LeafPedigree],
        now: SystemTime,
    ) {
        let mut backoff = self.backoff.lock().unwrap();
        for pedigree in resolved {
            backoff.remove(&pedigree.leaf.id);
        }
        for pedigree in unresolved {
            let delay = backoff
                .get(&pedigree.leaf.id)
                .map_or(INITIAL_RETRY_DELAY, |entry| {
                    (entry.delay * 2).min(MAX_RETRY_DELAY)
                });
            backoff.insert(
                pedigree.leaf.id.clone(),
                Backoff {
                    retry_at: now + delay,
                    delay,
                },
            );
        }
    }

    /// Drops the backoff of leaves that are no longer stored, so a wallet that keeps
    /// spending does not accumulate entries for ids it will never see again.
    fn retain_backoff(&self, present: &HashSet<TreeNodeId>) {
        let mut backoff = self.backoff.lock().unwrap();
        backoff.retain(|leaf_id, _| present.contains(leaf_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::tests::create_test_node_with_parent;
    use crate::tree::{
        LeafSelection, Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose,
        SelectLeavesOptions, TargetAmounts, TreeNode, TreeNodeStatus,
    };
    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn pedigree(leaf: &TreeNode, ancestors: Vec<TreeNode>) -> LeafPedigree {
        LeafPedigree {
            leaf: leaf.clone(),
            ancestors,
        }
    }

    #[derive(Default)]
    struct MockState {
        leaves: Vec<TreeNode>,
        /// What `load_exit_chains` returns for a leaf id, if seeded.
        stored_chains: HashMap<TreeNodeId, LeafPedigree>,
        /// What `fetch_pedigrees_from_operators` returns for a leaf id, if seeded.
        operator_responses: HashMap<TreeNodeId, LeafPedigree>,
        /// Leaf ids passed to each `fetch_pedigrees_from_operators` call, in order.
        fetch_calls: Vec<Vec<TreeNodeId>>,
        /// Pedigrees passed to `store_exit_chains`, accumulated across calls.
        stored: Vec<LeafPedigree>,
        /// One-shot gate the next `fetch_pedigrees_from_operators` call waits on
        /// before returning, so a test can hold a run "in progress".
        gate: Option<Arc<Notify>>,
    }

    /// Backs the three `TreeService` methods the resolver calls, recording each
    /// request so tests can assert on it. Every other method is unreachable from
    /// this worker and panics if called.
    #[derive(Default)]
    struct MockTreeService {
        state: Mutex<MockState>,
    }

    impl MockTreeService {
        fn seed_leaf(&self, leaf: TreeNode, stored_chain: LeafPedigree) {
            let mut state = self.state.lock().unwrap();
            state.stored_chains.insert(leaf.id.clone(), stored_chain);
            state.leaves.push(leaf);
        }

        fn set_operator_response(&self, leaf_id: TreeNodeId, pedigree: LeafPedigree) {
            self.state
                .lock()
                .unwrap()
                .operator_responses
                .insert(leaf_id, pedigree);
        }

        fn fetch_call_count(&self) -> usize {
            self.state.lock().unwrap().fetch_calls.len()
        }

        fn fetch_calls(&self) -> Vec<Vec<TreeNodeId>> {
            self.state.lock().unwrap().fetch_calls.clone()
        }

        fn stored(&self) -> Vec<LeafPedigree> {
            self.state.lock().unwrap().stored.clone()
        }

        /// Arms the gate for the next `fetch_pedigrees_from_operators` call and
        /// returns the handle used to release it.
        fn arm_gate(&self) -> Arc<Notify> {
            let gate = Arc::new(Notify::new());
            self.state.lock().unwrap().gate = Some(Arc::clone(&gate));
            gate
        }
    }

    #[macros::async_trait]
    impl TreeService for MockTreeService {
        async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn list_leaves(&self) -> Result<Leaves, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn fetch_nodes(
            &self,
            _node_ids: &[TreeNodeId],
            _include_parents: bool,
        ) -> Result<Vec<TreeNode>, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn load_exit_chains(
            &self,
            _leaf_ids: &[TreeNodeId],
        ) -> Result<Vec<LeafPedigree>, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn leaves_missing_exit_chains(&self) -> Result<Vec<TreeNodeId>, TreeServiceError> {
            let state = self.state.lock().unwrap();
            Ok(state
                .leaves
                .iter()
                .filter(|leaf| {
                    leaf.parent_node_id.is_some()
                        && !state.stored_chains.get(&leaf.id).is_some_and(is_complete)
                })
                .map(|leaf| leaf.id.clone())
                .collect())
        }

        async fn store_exit_chains(
            &self,
            pedigrees: &[LeafPedigree],
        ) -> Result<(), TreeServiceError> {
            self.state
                .lock()
                .unwrap()
                .stored
                .extend_from_slice(pedigrees);
            Ok(())
        }

        async fn fetch_pedigrees_from_operators(
            &self,
            leaf_ids: &[TreeNodeId],
        ) -> Vec<LeafPedigree> {
            let gate = {
                let mut state = self.state.lock().unwrap();
                state.fetch_calls.push(leaf_ids.to_vec());
                state.gate.take()
            };
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let state = self.state.lock().unwrap();
            leaf_ids
                .iter()
                .filter_map(|id| state.operator_responses.get(id).cloned())
                .collect()
        }

        async fn refresh_leaves(&self) -> Result<(), TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn insert_leaves(
            &self,
            _leaves: Vec<TreeNode>,
        ) -> Result<Vec<TreeNode>, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn select_leaves(
            &self,
            _target_amounts: Option<&TargetAmounts>,
            _purpose: ReservationPurpose,
            _options: SelectLeavesOptions,
        ) -> Result<LeavesReservation, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn reserve_leaves_by_ids(
            &self,
            _leaf_ids: &[TreeNodeId],
            _purpose: ReservationPurpose,
        ) -> Result<LeavesReservation, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn select_leaves_for_package(
            &self,
            _target_amounts: Option<&TargetAmounts>,
        ) -> Result<LeafSelection, TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn cancel_reservation(
            &self,
            _reservation: LeavesReservation,
        ) -> Result<(), TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }

        async fn finalize_reservation(
            &self,
            _id: LeavesReservationId,
            _new_leaves: Option<&[LeafPedigree]>,
        ) -> Result<(), TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }
    }

    /// Polls `condition` until it holds or `timeout` elapses, sleeping briefly
    /// between checks. Used in place of a fixed sleep so the coalescing test
    /// does not have to guess how long a run takes.
    async fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let step = Duration::from_millis(5);
        let mut waited = Duration::ZERO;
        while !condition() {
            if waited >= timeout {
                return false;
            }
            tokio::time::sleep(step).await;
            waited += step;
        }
        true
    }

    #[async_test_all]
    async fn test_incomplete_stored_chain_is_fetched_and_stored() {
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()));
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![root.clone()]));

        let resolver = ExitChainResolver::new(mock.clone(), &ExitChainTrigger::new());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(mock.fetch_calls(), vec![vec![leaf.id.clone()]]);
        let stored = mock.stored();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].leaf, leaf);
        assert_eq!(stored[0].ancestors, vec![root]);
    }

    #[async_test_all]
    async fn test_complete_stored_chain_is_not_fetched() {
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, vec![root]));

        let resolver = ExitChainResolver::new(mock.clone(), &ExitChainTrigger::new());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(mock.fetch_call_count(), 0);
        assert!(mock.stored().is_empty());
    }

    #[async_test_all]
    async fn test_root_leaf_is_treated_as_complete() {
        // A root leaf has no parent and needs no ancestors, so its stored chain
        // (seeded here with none) already satisfies `is_complete`.
        let root_leaf = create_test_node_with_parent("root-leaf", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(root_leaf.clone(), pedigree(&root_leaf, Vec::new()));

        let resolver = ExitChainResolver::new(mock.clone(), &ExitChainTrigger::new());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(mock.fetch_call_count(), 0);
        assert!(mock.stored().is_empty());
    }

    #[async_test_all]
    async fn test_repeated_failure_backs_off() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);
        let mid =
            create_test_node_with_parent("mid", Some("missing-root"), TreeNodeStatus::Splitted);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()));
        // The operators still can't complete the chain, so this leaf must back off.
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![mid]));

        let resolver = ExitChainResolver::new(mock.clone(), &ExitChainTrigger::new());

        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(mock.fetch_call_count(), 1);

        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(
            mock.fetch_call_count(),
            1,
            "leaf must not be re-requested before its retry delay elapses"
        );
    }

    #[async_test_all]
    async fn test_success_clears_earlier_backoff() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);
        let mid =
            create_test_node_with_parent("mid", Some("missing-root"), TreeNodeStatus::Splitted);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()));
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![mid]));

        let resolver = ExitChainResolver::new(mock.clone(), &ExitChainTrigger::new());
        resolver.resolve_missing_chains().await.unwrap();
        assert!(resolver.backoff.lock().unwrap().contains_key(&leaf.id));

        // The resolver's clock can't be mocked, so simulate the delay elapsing by
        // backdating the recorded retry time directly.
        {
            let mut backoff = resolver.backoff.lock().unwrap();
            backoff.get_mut(&leaf.id).unwrap().retry_at =
                SystemTime::now() - Duration::from_secs(1);
        }
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![root]));

        resolver.resolve_missing_chains().await.unwrap();
        assert!(!resolver.backoff.lock().unwrap().contains_key(&leaf.id));
    }

    #[async_test_all]
    async fn test_coalesced_trigger_causes_exactly_one_more_run() {
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        // The stored chain never reflects the fix, and the operators always
        // resolve it, so every pass is a fresh candidate and no backoff can
        // creep in to mask the coalescing behavior under test.
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()));
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![root]));

        let trigger = ExitChainTrigger::new();
        let resolver = Arc::new(ExitChainResolver::new(mock.clone(), &trigger));
        let (cancel_tx, cancel_rx) = watch::channel(());

        let run_resolver = Arc::clone(&resolver);
        let handle = tokio::spawn(async move {
            run_resolver.run(cancel_rx).await;
        });

        // `run` fetches once immediately at startup, without waiting for a trigger.
        assert!(
            wait_until(Duration::from_secs(5), || mock.fetch_call_count() >= 1).await,
            "expected the initial pass to run without a trigger"
        );
        let after_first_pass = mock.fetch_call_count();

        // Arm the gate and trigger once so the *next* pass starts and then
        // blocks partway through, giving a window where a run is genuinely
        // "in progress" (the resolver is not sitting idle at its own signal).
        let gate = mock.arm_gate();
        trigger.trigger();
        assert!(
            wait_until(Duration::from_secs(5), || mock.fetch_call_count()
                > after_first_pass)
            .await,
            "expected the trigger to start another pass"
        );

        // While that pass is blocked mid-flight, a burst of further triggers
        // must coalesce into exactly one more pass, not one per trigger.
        trigger.trigger();
        trigger.trigger();
        trigger.trigger();
        gate.notify_one();

        assert!(
            wait_until(Duration::from_secs(5), || mock.fetch_call_count()
                >= after_first_pass + 2)
            .await,
            "expected the coalesced trigger to cause exactly one further pass"
        );
        // Give a slow scheduler room to run any extra (incorrect) passes before
        // asserting the count settles instead of continuing to climb.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            mock.fetch_call_count(),
            after_first_pass + 2,
            "a burst of triggers during a run must cause exactly one more run"
        );

        let _ = cancel_tx.send(());
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("run() should return once cancelled")
            .expect("run() task should not panic");
    }
}
