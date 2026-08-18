//! Resolves the exit chains of stored leaves that lack one.
//!
//! A leaf is persisted as soon as the operation that produced it completes, without
//! its chain: resolving the chain costs an operator round trip, and an operation
//! that swaps often sends most of its new leaves straight back out, so the chains
//! of the ones that leave would be fetched and written for nothing. Fetching
//! afterwards pays that cost once, only for the leaves that stayed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use platform_utils::time::SystemTime;
use platform_utils::tokio::sync::Mutex;
use tracing::trace;

use crate::tree::{LeafPedigree, TreeNodeId, TreeService, TreeServiceError, chain_reaches_root};

/// Delay before a leaf whose chain the operators did not complete is tried again.
/// Doubles per consecutive failure up to [`MAX_RETRY_DELAY`].
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30 * 60);

/// Leaves whose chains are fetched, held and stored at a time. A chain carries a
/// node per level of the tree, each with its transactions, so the first pass over
/// a wallet with no chains stored yet would otherwise hold every one of them at
/// once. Bounds the id list one operator request carries as well. Tiny under
/// test, so an ordinary fixture spans several batches.
#[cfg(not(test))]
const LEAVES_PER_FETCH: usize = 500;
#[cfg(test)]
const LEAVES_PER_FETCH: usize = 2;

fn is_complete(pedigree: &LeafPedigree) -> bool {
    chain_reaches_root(&pedigree.leaf, &pedigree.ancestors)
}

#[derive(Clone, Copy)]
struct Backoff {
    retry_at: SystemTime,
    delay: Duration,
}

pub struct ExitChainResolver {
    tree_service: Arc<dyn TreeService>,
    /// Leaves the operators would not complete, held off until their retry time.
    /// Deliberately in memory only: a restart is a good moment to try again.
    backoff: Mutex<HashMap<TreeNodeId, Backoff>>,
}

impl ExitChainResolver {
    pub fn new(tree_service: Arc<dyn TreeService>) -> Self {
        Self {
            tree_service,
            backoff: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves what is missing, once.
    ///
    /// Leaves the operators recently failed to complete stay backed off, so calling
    /// this in a loop does not turn into a retry storm.
    pub async fn resolve_missing_chains(&self) -> Result<(), TreeServiceError> {
        self.resolve(true).await
    }

    /// Resolves what is missing, once, asking about every leaf whatever its
    /// backoff. Trades a possibly redundant round trip for the guarantee that
    /// nothing stays unresolved merely because it is inside a retry delay.
    ///
    /// A leaf already inside its delay keeps the schedule it had, so however
    /// often this runs it cannot re-pace [`Self::resolve_missing_chains`].
    pub async fn resolve_all_missing_chains(&self) -> Result<(), TreeServiceError> {
        self.resolve(false).await
    }

    async fn resolve(&self, honor_backoff: bool) -> Result<(), TreeServiceError> {
        let missing = self.tree_service.leaves_missing_exit_chains().await?;
        self.retain_backoff(&missing.iter().cloned().collect())
            .await;
        if missing.is_empty() {
            return Ok(());
        }

        let now = SystemTime::now();
        let candidates = if honor_backoff {
            self.due_leaves(missing, now).await
        } else {
            missing
        };
        if candidates.is_empty() {
            return Ok(());
        }
        trace!("Resolving exit chains for {} leaves", candidates.len());

        // One `now` for the whole pass, so a leaf's retry time does not depend on
        // which batch it landed in.
        for batch in candidates.chunks(LEAVES_PER_FETCH) {
            self.resolve_batch(batch, now).await?;
        }
        Ok(())
    }

    /// Fetches one batch of chains, stores those that came back complete, and
    /// paces the rest.
    async fn resolve_batch(
        &self,
        candidates: &[TreeNodeId],
        now: SystemTime,
    ) -> Result<(), TreeServiceError> {
        let fetched = self
            .tree_service
            .fetch_pedigrees_from_operators(candidates)
            .await;
        let returned: HashSet<TreeNodeId> = fetched.iter().map(|p| p.leaf.id.clone()).collect();
        let (resolved, unresolved): (Vec<LeafPedigree>, Vec<LeafPedigree>) =
            fetched.into_iter().partition(is_complete);

        // Only a chain that spans the leaf is stored. The stores take a stored
        // chain as one that backs an exit, so a partial one would read as
        // exitable and build a set that cannot confirm.
        if !resolved.is_empty() {
            self.tree_service.store_exit_chains(&resolved).await?;
        }

        // A candidate the operators left out of their response came back in no
        // pedigree at all, so it needs its backoff recorded from `candidates`.
        // Otherwise nothing paces a leaf they never answer about, and a fetch
        // that fails outright, which yields no pedigrees, would pace nothing.
        let failed: Vec<TreeNodeId> = unresolved
            .iter()
            .map(|pedigree| pedigree.leaf.id.clone())
            .chain(
                candidates
                    .iter()
                    .filter(|leaf_id| !returned.contains(leaf_id))
                    .cloned(),
            )
            .collect();

        self.record_outcomes(&resolved, &failed, now).await;
        Ok(())
    }

    /// The leaves worth asking the operators about now: everything except those
    /// still inside the retry delay a previous failure earned them.
    async fn due_leaves(&self, missing: Vec<TreeNodeId>, now: SystemTime) -> Vec<TreeNodeId> {
        let backoff = self.backoff.lock().await;
        missing
            .into_iter()
            .filter(|leaf_id| {
                backoff
                    .get(leaf_id)
                    .is_none_or(|entry| now >= entry.retry_at)
            })
            .collect()
    }

    /// Clears the backoff of leaves that resolved and extends it for those that
    /// did not, so a chain the operators cannot complete is not retried on every
    /// pass.
    async fn record_outcomes(
        &self,
        resolved: &[LeafPedigree],
        failed: &[TreeNodeId],
        now: SystemTime,
    ) {
        let mut backoff = self.backoff.lock().await;
        for pedigree in resolved {
            backoff.remove(&pedigree.leaf.id);
        }
        for leaf_id in failed {
            // A leaf still inside its delay was only reached by a pass that
            // ignored the backoff. Leaving its schedule alone is what stops the
            // delay growing with how often such a pass is run rather than with
            // how long the leaf has actually been failing.
            if backoff
                .get(leaf_id)
                .is_some_and(|entry| now < entry.retry_at)
            {
                continue;
            }
            let delay = backoff.get(leaf_id).map_or(INITIAL_RETRY_DELAY, |entry| {
                (entry.delay * 2).min(MAX_RETRY_DELAY)
            });
            backoff.insert(
                leaf_id.clone(),
                Backoff {
                    retry_at: now + delay,
                    delay,
                },
            );
        }
    }

    /// Drops the backoff of leaves that are no longer stored, so a wallet that keeps
    /// spending does not accumulate entries for ids it will never see again.
    async fn retain_backoff(&self, present: &HashSet<TreeNodeId>) {
        let mut backoff = self.backoff.lock().await;
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
    }

    /// Backs the three `TreeService` methods the resolver calls, recording each
    /// request so tests can assert on it. Every other method is unreachable from
    /// this worker and panics if called.
    #[derive(Default)]
    struct MockTreeService {
        state: Mutex<MockState>,
    }

    impl MockTreeService {
        async fn seed_leaf(&self, leaf: TreeNode, stored_chain: LeafPedigree) {
            let mut state = self.state.lock().await;
            state.stored_chains.insert(leaf.id.clone(), stored_chain);
            state.leaves.push(leaf);
        }

        async fn set_operator_response(&self, leaf_id: TreeNodeId, pedigree: LeafPedigree) {
            self.state
                .lock()
                .await
                .operator_responses
                .insert(leaf_id, pedigree);
        }

        async fn fetch_call_count(&self) -> usize {
            self.state.lock().await.fetch_calls.len()
        }

        async fn fetch_calls(&self) -> Vec<Vec<TreeNodeId>> {
            self.state.lock().await.fetch_calls.clone()
        }

        async fn stored(&self) -> Vec<LeafPedigree> {
            self.state.lock().await.stored.clone()
        }
    }

    #[macros::async_trait]
    impl TreeService for MockTreeService {
        fn subscribe_leaves_added(&self) -> platform_utils::tokio::sync::broadcast::Receiver<()> {
            unimplemented!("not exercised by ExitChainResolver")
        }

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
            let state = self.state.lock().await;
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
            let mut state = self.state.lock().await;
            state.stored.extend_from_slice(pedigrees);
            // A stored chain is what stops a leaf being reported missing, so
            // without this the resolver would keep refetching the same leaf.
            for pedigree in pedigrees {
                state
                    .stored_chains
                    .insert(pedigree.leaf.id.clone(), pedigree.clone());
            }
            Ok(())
        }

        async fn fetch_pedigrees_from_operators(
            &self,
            leaf_ids: &[TreeNodeId],
        ) -> Vec<LeafPedigree> {
            self.state.lock().await.fetch_calls.push(leaf_ids.to_vec());
            let state = self.state.lock().await;
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
            _new_leaves: Option<&[TreeNode]>,
        ) -> Result<(), TreeServiceError> {
            unimplemented!("not exercised by ExitChainResolver")
        }
    }
    #[async_test_all]
    async fn test_incomplete_stored_chain_is_fetched_and_stored() {
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![root.clone()]))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(mock.fetch_calls().await, vec![vec![leaf.id.clone()]]);
        let stored = mock.stored().await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].leaf, leaf);
        assert_eq!(stored[0].ancestors, vec![root]);
    }

    #[async_test_all]
    async fn test_candidates_are_resolved_in_batches() {
        // A wallet with no chains stored yet reports every leaf at once, and each
        // chain fetched carries a node per level of the tree. Fetching and storing
        // in batches is what keeps that off the heap all at once.
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);
        let mock = Arc::new(MockTreeService::default());
        let mut ids = Vec::new();
        for name in ["leaf-a", "leaf-b", "leaf-c"] {
            let leaf = create_test_node_with_parent(name, Some("root"), TreeNodeStatus::Available);
            mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
                .await;
            mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![root.clone()]))
                .await;
            ids.push(leaf.id);
        }

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(
            mock.fetch_calls().await,
            vec![vec![ids[0].clone(), ids[1].clone()], vec![ids[2].clone()]]
        );
        // Every leaf still ends up resolved: batching splits the work, it does not
        // drop any of it.
        assert_eq!(mock.stored().await.len(), 3);
    }

    #[async_test_all]
    async fn test_complete_stored_chain_is_not_fetched() {
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, vec![root]))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(mock.fetch_call_count().await, 0);
        assert!(mock.stored().await.is_empty());
    }

    #[async_test_all]
    async fn test_chain_stopping_short_of_a_root_is_not_stored() {
        // The operators answer with the leaf's parent but nothing above it. Storing
        // that would leave the leaf reading as complete for good, since a stored
        // chain is only ever checked as far as the parent.
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);
        let mid = create_test_node_with_parent("mid", Some("root"), TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![mid]))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();

        assert!(mock.stored().await.is_empty());
    }

    #[async_test_all]
    async fn test_root_leaf_is_treated_as_complete() {
        // A root leaf has no parent and needs no ancestors, so its stored chain
        // (seeded here with none) already satisfies `is_complete`.
        let root_leaf = create_test_node_with_parent("root-leaf", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(root_leaf.clone(), pedigree(&root_leaf, Vec::new()))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();

        assert_eq!(mock.fetch_call_count().await, 0);
        assert!(mock.stored().await.is_empty());
    }

    #[async_test_all]
    async fn test_repeated_failure_backs_off() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        // The operators return the leaf but none of its ancestry, so the chain
        // still does not reach its parent and the leaf must back off.
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, Vec::new()))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());

        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(mock.fetch_call_count().await, 1);

        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(
            mock.fetch_call_count().await,
            1,
            "leaf must not be re-requested before its retry delay elapses"
        );
    }

    #[async_test_all]
    async fn test_leaf_the_operators_omit_backs_off() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        // No operator response seeded: the leaf is left out of the reply, exactly
        // as a whole failed fetch leaves out every leaf it was asked about.

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(mock.fetch_call_count().await, 1);

        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(
            mock.fetch_call_count().await,
            1,
            "a leaf absent from the reply must back off like one that came back incomplete"
        );
    }

    #[async_test_all]
    async fn test_resolving_all_ignores_backoff() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, Vec::new()))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();
        assert_eq!(mock.fetch_call_count().await, 1);
        assert!(resolver.backoff.lock().await.contains_key(&leaf.id));

        resolver.resolve_all_missing_chains().await.unwrap();
        assert_eq!(
            mock.fetch_call_count().await,
            2,
            "the exit path asks again inside the retry delay"
        );
    }

    #[async_test_all]
    async fn test_resolving_all_does_not_escalate_a_live_backoff() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, Vec::new()))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();
        let earned = *resolver.backoff.lock().await.get(&leaf.id).unwrap();

        // However many times the exit path runs, the delay the periodic path
        // will honour must not grow: it tracks how long the leaf has been
        // failing, not how often someone asked about it.
        for _ in 0..4 {
            resolver.resolve_all_missing_chains().await.unwrap();
        }
        let after = *resolver.backoff.lock().await.get(&leaf.id).unwrap();
        assert_eq!(after.delay, earned.delay, "delay must not grow per pass");
        assert_eq!(after.retry_at, earned.retry_at, "retry must not slide out");
    }

    #[async_test_all]
    async fn test_success_clears_earlier_backoff() {
        let leaf = create_test_node_with_parent("leaf", Some("mid"), TreeNodeStatus::Available);
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Available);

        let mock = Arc::new(MockTreeService::default());
        mock.seed_leaf(leaf.clone(), pedigree(&leaf, Vec::new()))
            .await;
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, Vec::new()))
            .await;

        let resolver = ExitChainResolver::new(mock.clone());
        resolver.resolve_missing_chains().await.unwrap();
        assert!(resolver.backoff.lock().await.contains_key(&leaf.id));

        // The resolver's clock can't be mocked, so simulate the delay elapsing by
        // backdating the recorded retry time directly.
        {
            let mut backoff = resolver.backoff.lock().await;
            backoff.get_mut(&leaf.id).unwrap().retry_at =
                SystemTime::now() - Duration::from_secs(1);
        }
        // This time the operators return the chain, so it reaches the parent.
        let mid_rooted =
            create_test_node_with_parent("mid", Some("root"), TreeNodeStatus::Splitted);
        mock.set_operator_response(leaf.id.clone(), pedigree(&leaf, vec![mid_rooted, root]))
            .await;

        resolver.resolve_missing_chains().await.unwrap();
        assert!(!resolver.backoff.lock().await.contains_key(&leaf.id));
    }
}
