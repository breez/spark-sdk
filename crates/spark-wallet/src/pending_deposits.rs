//! Deposits the wallet watches until every operator holds the leaf as available.
//!
//! Each operator marks a deposit's leaf available when its own chain watcher
//! processes the block that contains the deposit transaction, so the coordinator
//! can hold the leaf as available before the other operators do. They reject a
//! transfer of a leaf they still hold as creating. The wallet therefore refreshes
//! a deposit's leaf on its own: the tree service stores the leaf only once every
//! operator holds it as available, and the wallet then announces the deposit.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use platform_utils::time::Instant;
use platform_utils::tokio;
use platform_utils::tokio::sync::{Notify, watch};
use spark::tree::{TreeNode, TreeNodeId, TreeNodeStatus, TreeService, TreeServiceError};
use tracing::{error, info, warn};

use crate::WalletEvent;
use crate::event::EventManager;

/// The delays between the checks the wallet runs while the operators catch up.
/// The first delay is short, because they normally agree within seconds, and
/// the wallet doubles each delay after that, up to `max_delay`. After
/// `give_up_after` the wallet stops checking these deposits on a timer and
/// checks them after each sync instead.
#[derive(Clone, Copy)]
struct Backoff {
    base_delay: Duration,
    max_delay: Duration,
    give_up_after: Duration,
}

impl Backoff {
    const fn new() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            give_up_after: Duration::from_secs(120),
        }
    }
}

/// The wallet's place in its backoff: when the wallet started the backoff, and
/// when it checks next.
#[derive(Clone, Copy)]
struct Checking {
    started: Instant,
    delay: Duration,
    next_check: Instant,
}

impl Checking {
    fn start(backoff: &Backoff) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            delay: backoff.base_delay,
            next_check: now + backoff.base_delay,
        }
    }

    /// The next step of the backoff, or `None` once `give_up_after` has passed
    /// since `started`.
    fn next(self, backoff: &Backoff) -> Option<Self> {
        if self.started.elapsed() >= backoff.give_up_after {
            return None;
        }
        let delay = (self.delay * 2).min(backoff.max_delay);
        Some(Self {
            started: self.started,
            delay,
            next_check: Instant::now() + delay,
        })
    }
}

/// The deposits whose leaf one operator holds as available, which the wallet
/// announces once every operator holds it.
pub(crate) struct PendingDeposits {
    tree_service: Arc<dyn TreeService>,
    events: Arc<EventManager>,
    /// The leaves of the deposits the wallet has yet to announce. The wallet
    /// holds them in memory only, so after a restart it does not announce a
    /// deposit that was still pending when the wallet stopped.
    pending: Mutex<HashSet<TreeNodeId>>,
    /// The wallet raises this when it adds a pending deposit.
    deposit_added: Notify,
    /// The wallet raises this when it has synced.
    synced: Notify,
    backoff: Backoff,
}

impl PendingDeposits {
    pub(crate) fn new(tree_service: Arc<dyn TreeService>, events: Arc<EventManager>) -> Self {
        Self::with_backoff(tree_service, events, Backoff::new())
    }

    fn with_backoff(
        tree_service: Arc<dyn TreeService>,
        events: Arc<EventManager>,
        backoff: Backoff,
    ) -> Self {
        Self {
            tree_service,
            events,
            pending: Mutex::new(HashSet::new()),
            deposit_added: Notify::new(),
            synced: Notify::new(),
            backoff,
        }
    }

    /// Takes the leaves a caller just claimed. When the coordinator holds a leaf
    /// as creating, the deposit transaction has yet to confirm, and the
    /// coordinator reports the leaf once the transaction confirms. When the
    /// coordinator holds a leaf as available, the transaction confirmed before
    /// the claim, and the coordinator sends no report. The wallet then checks the
    /// leaf with every operator at once, and keeps watching the deposit if they
    /// do not all hold the leaf as available yet. Returns the check's error, if
    /// any.
    pub(crate) async fn claimed(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
        let confirmed: Vec<TreeNodeId> = leaves
            .iter()
            .filter(|leaf| leaf.status == TreeNodeStatus::Available)
            .map(|leaf| leaf.id.clone())
            .collect();
        if confirmed.is_empty() {
            return Ok(());
        }
        self.lock().extend(confirmed);
        let checked = self.check().await;
        if !self.lock().is_empty() {
            self.deposit_added.notify_one();
        }
        checked
    }

    /// Takes the leaf the coordinator reported available once its chain watcher
    /// saw the deposit transaction confirm. The other operators mark the leaf
    /// available shortly after.
    pub(crate) fn reported(&self, leaf: TreeNode) {
        self.lock().insert(leaf.id);
        self.deposit_added.notify_one();
    }

    /// Notes that the wallet synced, so the background task checks the pending
    /// deposits again. Once the backoff is over, the wallet checks a deposit
    /// only then.
    pub(crate) fn synced(&self) {
        self.synced.notify_one();
    }

    /// Checks the pending deposits on the backoff, and after each sync, until
    /// background processing stops. It also watches the deposits added before
    /// it started.
    pub(crate) async fn run(&self, mut cancel: watch::Receiver<()>) {
        let mut checking = (!self.lock().is_empty()).then(|| Checking::start(&self.backoff));
        loop {
            let next_check = checking.map(|checking| checking.next_check);
            tokio::select! {
                () = self.deposit_added.notified() => {
                    checking = Some(Checking::start(&self.backoff));
                }
                () = self.synced.notified() => self.check_or_log().await,
                () = sleep_until(next_check) => {
                    checking = checking.and_then(|checking| checking.next(&self.backoff));
                    self.check_or_log().await;
                    if self.lock().is_empty() {
                        checking = None;
                    } else if checking.is_none() {
                        self.log_still_pending();
                    }
                }
                _ = cancel.changed() => return,
            }
        }
    }

    async fn check_or_log(&self) {
        if let Err(e) = self.check().await {
            error!("could not check the pending deposits with the operators: {e:?}");
        }
    }

    /// Refreshes the pending deposits' leaves, and announces each deposit whose
    /// leaf the tree service stored. Stops watching a leaf the coordinator no
    /// longer holds as available, such as one the wallet has spent or exited.
    async fn check(&self) -> Result<(), TreeServiceError> {
        let pending = self.lock().clone();
        if pending.is_empty() {
            return Ok(());
        }
        let leaf_ids: Vec<TreeNodeId> = pending.iter().cloned().collect();
        let refreshed = self.tree_service.refresh_leaves_by_ids(&leaf_ids).await?;
        let available: HashSet<TreeNodeId> = refreshed
            .available
            .into_iter()
            .map(|leaf| leaf.id)
            .collect();
        let missing: HashSet<TreeNodeId> = refreshed
            .available_missing_from_operators
            .into_iter()
            .map(|leaf| leaf.id)
            .collect();

        let mut announced = Vec::new();
        self.lock().retain(|leaf_id| {
            // A deposit added during this check waits for the next one.
            if !pending.contains(leaf_id) || missing.contains(leaf_id) {
                return true;
            }
            if available.contains(leaf_id) {
                announced.push(leaf_id.clone());
            } else {
                info!(
                    "the coordinator no longer holds deposit leaf {leaf_id} as available; \
                     no longer watching"
                );
            }
            false
        });
        for leaf_id in announced {
            info!("every operator has deposit leaf {leaf_id} available");
            self.events
                .notify_listeners(WalletEvent::DepositConfirmed(leaf_id));
        }
        Ok(())
    }

    fn log_still_pending(&self) {
        let leaf_ids: Vec<String> = self.lock().iter().map(ToString::to_string).collect();
        warn!(
            "deposit leaves {} are still pending; the wallet stops checking them on a timer \
             and checks again after each sync",
            leaf_ids.join(", ")
        );
    }

    /// The leaves of the pending deposits.
    #[cfg(test)]
    fn pending_leaves(&self) -> Vec<TreeNodeId> {
        self.lock().iter().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<TreeNodeId>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => {
            tokio::time::sleep(deadline.saturating_duration_since(Instant::now())).await;
        }
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::MutexGuard;

    use ::tokio::sync::broadcast;
    use spark::tree::tests::create_test_tree_node;
    use spark::tree::{MockTreeService, RefreshedLeaves};

    use super::*;

    /// What the operators hold, which a test sets up, and what the mocked tree
    /// service stores.
    #[derive(Default)]
    struct Tree {
        held_by_every_operator: Vec<TreeNode>,
        held_by_one_operator: Vec<TreeNode>,
        unreachable: bool,
        stored: Vec<TreeNode>,
        checks: usize,
    }

    impl Tree {
        /// What the tree service does when it refreshes `leaf_ids`: it stores
        /// each leaf every operator holds as available.
        fn refresh(
            &mut self,
            leaf_ids: &[TreeNodeId],
        ) -> Result<RefreshedLeaves, TreeServiceError> {
            self.checks += 1;
            if self.unreachable {
                return Err(TreeServiceError::Generic(
                    "operators unreachable".to_string(),
                ));
            }
            let requested = |leaves: &[TreeNode]| -> Vec<TreeNode> {
                leaves
                    .iter()
                    .filter(|leaf| leaf_ids.contains(&leaf.id))
                    .cloned()
                    .collect()
            };
            let stored = requested(&self.held_by_every_operator);
            self.stored.extend(stored.iter().cloned());
            Ok(RefreshedLeaves {
                available: stored,
                available_missing_from_operators: requested(&self.held_by_one_operator),
            })
        }
    }

    /// A tree service that reads and writes `tree`.
    fn tree_service(tree: &Arc<Mutex<Tree>>) -> Arc<dyn TreeService> {
        let mut mock = MockTreeService::new();
        let tree = Arc::clone(tree);
        mock.expect_refresh_leaves_by_ids()
            .returning(move |leaf_ids| lock(&tree).refresh(leaf_ids));
        Arc::new(mock)
    }

    fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
        value.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Short enough for a test to wait out the whole backoff.
    const BACKOFF: Backoff = Backoff {
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(10),
        give_up_after: Duration::from_millis(60),
    };

    /// Long enough for the background task to act on anything a test hands it.
    const WAIT: Duration = Duration::from_secs(2);

    /// A claimed deposit's leaf: the root of its tree, as the coordinator holds
    /// it once the deposit transaction has confirmed.
    fn deposit_leaf(id: &str) -> TreeNode {
        create_test_tree_node(id, 10_000)
    }

    fn deposits(
        tree: &Arc<Mutex<Tree>>,
    ) -> (Arc<PendingDeposits>, broadcast::Receiver<WalletEvent>) {
        let events = Arc::new(EventManager::new());
        let announced = events.listen();
        let deposits = PendingDeposits::with_backoff(tree_service(tree), events, BACKOFF);
        (Arc::new(deposits), announced)
    }

    /// Runs the background task until the caller drops the returned sender.
    fn run(deposits: &Arc<PendingDeposits>) -> watch::Sender<()> {
        let (cancel, cancelled) = watch::channel(());
        let deposits = Arc::clone(deposits);
        platform_utils::spawn(async move { deposits.run(cancelled).await });
        cancel
    }

    async fn next_announcement(announced: &mut broadcast::Receiver<WalletEvent>) -> TreeNodeId {
        match tokio::time::timeout(WAIT, announced.recv()).await {
            Ok(Ok(WalletEvent::DepositConfirmed(id))) => id,
            other => panic!("expected a deposit announcement, got {other:?}"),
        }
    }

    /// The operators come to agree on the leaf after `tree` first shows it held
    /// by one operator alone.
    fn agree(tree: &Arc<Mutex<Tree>>, leaf: &TreeNode) {
        let mut tree = lock(tree);
        tree.held_by_one_operator = Vec::new();
        tree.held_by_every_operator = vec![leaf.clone()];
    }

    #[macros::async_test_all]
    async fn the_wallet_announces_a_reported_deposit_once_every_operator_has_it() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let _running = run(&deposits);
        let leaf = deposit_leaf("a");

        lock(&tree).held_by_every_operator = vec![leaf.clone()];
        deposits.reported(leaf.clone());

        assert_eq!(next_announcement(&mut announced).await, leaf.id);
        assert!(deposits.pending_leaves().is_empty());
        assert_eq!(lock(&tree).stored, vec![leaf]);
    }

    /// The wallet stops checking on a timer while one operator alone has the
    /// leaf, and announces the deposit when it checks again after a sync.
    #[macros::async_test_all]
    async fn the_wallet_announces_a_deposit_after_a_sync_once_the_backoff_is_over() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let _running = run(&deposits);
        let leaf = deposit_leaf("a");

        lock(&tree).held_by_one_operator = vec![leaf.clone()];
        deposits.reported(leaf.clone());
        tokio::time::sleep(BACKOFF.give_up_after * 3).await;
        assert!(announced.try_recv().is_err(), "announced on one operator");
        assert_eq!(deposits.pending_leaves(), vec![leaf.id.clone()]);

        agree(&tree, &leaf);
        deposits.synced();

        assert_eq!(next_announcement(&mut announced).await, leaf.id);
    }

    /// A failed check leaves the deposit pending, and the wallet announces it
    /// when a later check succeeds.
    #[macros::async_test_all]
    async fn the_wallet_keeps_watching_a_deposit_while_its_checks_fail() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let _running = run(&deposits);
        let leaf = deposit_leaf("a");

        {
            let mut tree = lock(&tree);
            tree.unreachable = true;
            tree.held_by_every_operator = vec![leaf.clone()];
        }
        deposits.reported(leaf.clone());
        tokio::time::sleep(BACKOFF.give_up_after * 3).await;
        assert!(lock(&tree).checks > 0, "no check ran");
        assert!(announced.try_recv().is_err(), "announced without a check");
        assert_eq!(deposits.pending_leaves(), vec![leaf.id.clone()]);

        lock(&tree).unreachable = false;
        deposits.synced();

        assert_eq!(next_announcement(&mut announced).await, leaf.id);
    }

    #[macros::async_test_all]
    async fn the_wallet_announces_at_once_a_claimed_deposit_every_operator_has() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let leaf = deposit_leaf("a");

        lock(&tree).held_by_every_operator = vec![leaf.clone()];
        deposits
            .claimed(std::slice::from_ref(&leaf))
            .await
            .expect("check");

        match announced.try_recv() {
            Ok(WalletEvent::DepositConfirmed(id)) => assert_eq!(id, leaf.id),
            other => panic!("expected a deposit announcement, got {other:?}"),
        }
        assert_eq!(lock(&tree).stored, vec![leaf]);
    }

    /// When the operators still disagree at the claim, the wallet stores nothing
    /// yet, and stores the leaf and announces the deposit once they agree.
    #[macros::async_test_all]
    async fn the_wallet_announces_a_claimed_deposit_once_the_operators_agree() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let _running = run(&deposits);
        let leaf = deposit_leaf("a");

        lock(&tree).held_by_one_operator = vec![leaf.clone()];
        deposits
            .claimed(std::slice::from_ref(&leaf))
            .await
            .expect("check");
        assert!(announced.try_recv().is_err(), "announced on one operator");
        assert!(lock(&tree).stored.is_empty(), "stored on one operator");

        agree(&tree, &leaf);

        assert_eq!(next_announcement(&mut announced).await, leaf.id);
        assert_eq!(lock(&tree).stored, vec![leaf]);
    }

    /// For a deposit claimed before its transaction confirmed, the wallet waits
    /// for the coordinator's report: until then no operator has the leaf
    /// available.
    #[macros::async_test_all]
    async fn the_wallet_leaves_an_unconfirmed_claim_to_the_coordinators_report() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, _announced) = deposits(&tree);
        let mut leaf = deposit_leaf("a");
        leaf.status = TreeNodeStatus::Creating;

        deposits
            .claimed(std::slice::from_ref(&leaf))
            .await
            .expect("no check");

        assert!(deposits.pending_leaves().is_empty());
        assert_eq!(lock(&tree).checks, 0);
    }

    /// The wallet holds on to a deposit it claims before background processing
    /// starts, and the background task watches it from the moment the task
    /// starts.
    #[macros::async_test_all]
    async fn the_wallet_watches_a_deposit_claimed_before_the_background_task_starts() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let leaf = deposit_leaf("a");

        lock(&tree).held_by_one_operator = vec![leaf.clone()];
        deposits
            .claimed(std::slice::from_ref(&leaf))
            .await
            .expect("check");
        agree(&tree, &leaf);
        let _running = run(&deposits);

        assert_eq!(next_announcement(&mut announced).await, leaf.id);
    }

    #[macros::async_test_all]
    async fn the_wallet_stops_watching_a_deposit_the_coordinator_no_longer_holds() {
        let tree = Arc::new(Mutex::new(Tree::default()));
        let (deposits, mut announced) = deposits(&tree);
        let leaf = deposit_leaf("a");

        deposits.reported(leaf);
        deposits.check().await.expect("check");

        assert!(announced.try_recv().is_err());
        assert!(deposits.pending_leaves().is_empty());
    }
}
