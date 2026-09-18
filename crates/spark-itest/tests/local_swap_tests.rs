use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use spark::operator::OperatorPool;
use spark::operator::rpc::{ConnectionManager, DefaultConnectionManager};
use spark::services::{Swap, TimelockManager, TransferService};
use spark::session_store::InMemorySessionStore;
use spark::signer::{Signer, SparkSigner, SparkSignerAdapter};
use spark::ssp::ServiceProvider;
use spark::tree::{
    InMemoryTreeStore, SynchronousTreeService, TreeNode, TreeNodeId, TreeService, TreeStore,
};
use spark::utils::leaf_key_tweak::with_node_id_keys;
use spark_itest::fixtures::sspd::internal_api;
use spark_itest::{
    fixtures::setup::{TestFixtures, create_test_signer_alice, create_test_signer_bob},
    helpers::deposit_with_amount,
};
use spark_wallet::{
    RetryConfig, ServiceProviderConfig, SparkWallet, SparkWalletConfig, WalletEvent,
};
use tracing::info;

struct SwapFixture {
    pub fixtures: TestFixtures,
    pub alice_wallet: SparkWallet,
    pub alice_signer: Arc<dyn Signer>,
    pub alice_config: SparkWalletConfig,
    pub ssp_config: ServiceProviderConfig,
}

impl SwapFixture {
    async fn swaps(&self) -> Result<Vec<internal_api::Swap>> {
        self.fixtures.sspd().await?.swaps().await
    }
}

async fn client_swap_leaves(
    config: &SparkWalletConfig,
    signer: Arc<dyn Signer>,
    ssp_config: &ServiceProviderConfig,
    leaf_ids: &[TreeNodeId],
    target_amounts: Option<Vec<u64>>,
) -> Result<Vec<TreeNode>> {
    let network = config.network;
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let identity_public_key = spark::signer::derive_identity_public_key(signer.as_ref()).await?;

    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn ConnectionManager> = Arc::new(DefaultConnectionManager::new());
    let operator_pool = Arc::new(
        OperatorPool::connect(
            &config.operator_pool,
            connection_manager,
            session_store.clone(),
            spark_signer.clone(),
            None,
        )
        .await?,
    );

    let transfer_service = Arc::new(TransferService::new(
        spark_signer.clone(),
        network,
        config.split_secret_threshold,
        operator_pool.clone(),
        None,
    ));

    let service_provider = Arc::new(ServiceProvider::new(
        ssp_config.clone(),
        spark_signer.clone(),
        session_store.clone(),
        None,
    )?);

    let swap_service = Arc::new(Swap::new(
        network,
        operator_pool.clone(),
        spark_signer.clone(),
        service_provider,
        transfer_service,
    ));

    let timelock_manager = Arc::new(TimelockManager::new(
        spark_signer.clone(),
        network,
        operator_pool.clone(),
    ));
    let tree_store: Arc<dyn TreeStore> = Arc::new(InMemoryTreeStore::default());
    let tree_service = SynchronousTreeService::new(
        identity_public_key,
        network,
        operator_pool.clone(),
        tree_store,
        timelock_manager,
        spark_signer.clone(),
        Some(swap_service.clone()),
        None,
    );

    tree_service.refresh_leaves().await?;
    let leaves = tree_service.list_leaves().await?;
    let mut nodes = Vec::with_capacity(leaf_ids.len());
    for id in leaf_ids {
        let node = leaves
            .available
            .iter()
            .find(|n| n.id == *id)
            .ok_or_else(|| anyhow::anyhow!("leaf not available for swap: {id}"))?;
        nodes.push(node.clone());
    }

    Ok(swap_service
        .swap_leaves(&with_node_id_keys(nodes), target_amounts)
        .await?)
}

async fn setup_swap_fixture() -> Result<SwapFixture> {
    let fixtures = TestFixtures::new().await?;
    let sspd = fixtures.sspd().await?;
    sspd.wait_for_pool(&fixtures.bitcoind, 1, Duration::from_secs(600))
        .await
        .context("waiting for the daemon to stock its pool")?;

    let ssp_config = ServiceProviderConfig {
        base_url: sspd.base_url.clone(),
        schema_endpoint: Some("graphql/spark/rc".to_string()),
        identity_public_key: sspd.identity_public_key,
        user_agent: Some("spark-itest/0.1.0".to_string()),
        retry_config: RetryConfig::default(),
    };
    let alice_config = fixtures
        .create_wallet_config_with_ssp(Some(ssp_config.clone()))
        .await?;
    let alice_signer: Arc<dyn Signer> = Arc::new(create_test_signer_alice());
    let alice_spark_signer: Arc<dyn SparkSigner> =
        Arc::new(SparkSignerAdapter::new(alice_signer.clone()));
    let alice_wallet = SparkWallet::connect(alice_config.clone(), alice_spark_signer).await?;

    let mut alice_listener = alice_wallet.subscribe_events();
    alice_wallet.start_background_processing().await;
    loop {
        let event = alice_listener.recv().await?;
        if matches!(event, WalletEvent::Synced) {
            break;
        }
    }

    Ok(SwapFixture {
        fixtures,
        alice_wallet,
        alice_signer,
        alice_config,
        ssp_config,
    })
}

#[tokio::test]
#[test_log::test]
async fn test_swap_from_tree_pool() -> Result<()> {
    let fixture = setup_swap_fixture().await?;
    let alice = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_with_amount(alice, bitcoind, 3072).await?;
    let alice_balance = alice.get_balance().await?;
    info!("Alice balance before swap: {} sats", alice_balance);
    assert_eq!(alice_balance, 3072);

    let leaves_before = alice.list_leaves().await?;
    assert_eq!(leaves_before.available.len(), 1);
    let leaf_to_swap = leaves_before.available[0].id.clone();

    info!("Starting swap...");
    let claimed_nodes = client_swap_leaves(
        &fixture.alice_config,
        fixture.alice_signer.clone(),
        &fixture.ssp_config,
        &[leaf_to_swap],
        None,
    )
    .await?;
    alice.sync().await?;
    info!(
        "Swap completed. Claimed {} leaves: {:?}",
        claimed_nodes.len(),
        claimed_nodes.iter().map(|n| n.value).collect::<Vec<_>>()
    );

    let alice_balance_after = alice.get_balance().await?;
    info!("Alice balance after swap: {} sats", alice_balance_after);
    assert_eq!(alice_balance_after, 3072);

    let leaves_after = alice.list_leaves().await?;
    let mut values: Vec<u64> = leaves_after.available.iter().map(|l| l.value).collect();
    values.sort_unstable();
    info!("Alice leaves after swap: {:?}", values);
    assert_eq!(values, vec![1024, 2048]);

    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_swap_ssp_claims_user_leaves() -> Result<()> {
    let fixture = setup_swap_fixture().await?;
    let alice = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_with_amount(alice, bitcoind, 3072).await?;
    let leaves_before = alice.list_leaves().await?;
    assert_eq!(leaves_before.available.len(), 1);
    let leaf_to_swap = leaves_before.available[0].id.clone();

    client_swap_leaves(
        &fixture.alice_config,
        fixture.alice_signer.clone(),
        &fixture.ssp_config,
        &[leaf_to_swap],
        None,
    )
    .await?;

    let inbound = 'claim: {
        for _ in 0..30 {
            let swaps = fixture.swaps().await?;
            if let Some(swap) = swaps.iter().find(|swap| !swap.inbound.is_empty()) {
                break 'claim swap.inbound.clone();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        panic!("the daemon did not claim Alice's leaf within the timeout");
    };

    let claimed_total: u64 = inbound.iter().map(|leaf| leaf.value_sats).sum();
    assert_eq!(inbound.len(), 1, "the daemon takes Alice's single leaf");
    assert_eq!(claimed_total, 3072, "the daemon takes 3072 sats from Alice");

    // A claimed leaf is admitted to the pool separately, after its refund
    // timelock is checked.
    let claimed_id: Vec<String> = inbound.iter().map(|l| l.leaf_id.clone()).collect();
    let store = fixture.fixtures.sspd().await?.tree_store().await?;
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let pooled = store.get_leaves().await?;
        if pooled
            .available
            .iter()
            .any(|l| claimed_id.contains(&l.id.to_string()))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the leaf the daemon claimed never joined the pool"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn test_swap_power_of_two_from_tree_pool() -> Result<()> {
    let fixture = setup_swap_fixture().await?;
    let alice = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_with_amount(alice, bitcoind, 4096).await?;
    assert_eq!(alice.get_balance().await?, 4096);

    let leaves_before = alice.list_leaves().await?;
    let leaf_to_swap = leaves_before.available[0].id.clone();

    let claimed_nodes = client_swap_leaves(
        &fixture.alice_config,
        fixture.alice_signer.clone(),
        &fixture.ssp_config,
        &[leaf_to_swap],
        None,
    )
    .await?;
    alice.sync().await?;
    info!(
        "Swap completed. Claimed {} leaves: {:?}",
        claimed_nodes.len(),
        claimed_nodes.iter().map(|n| n.value).collect::<Vec<_>>()
    );

    assert_eq!(alice.get_balance().await?, 4096);

    let leaves_after = alice.list_leaves().await?;
    assert_eq!(leaves_after.available.len(), 1);
    assert_eq!(leaves_after.available[0].value, 4096);

    Ok(())
}

/// The swapped amount decomposes into denominations above the largest the daemon
/// stocks.
#[tokio::test]
#[test_log::test]
async fn test_swap_insufficient_tree_pool() -> Result<()> {
    let fixture = setup_swap_fixture().await?;
    let alice = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    let beyond_the_pool = 2 * pool_ceiling_sats();
    deposit_with_amount(alice, bitcoind, beyond_the_pool).await?;

    let leaf_to_swap = alice.list_leaves().await?.available[0].id.clone();
    let result = client_swap_leaves(
        &fixture.alice_config,
        fixture.alice_signer.clone(),
        &fixture.ssp_config,
        &[leaf_to_swap],
        None,
    )
    .await;

    let error = result.err().context("a swap beyond the pool must fail")?;
    info!("got the expected refusal: {error:?}");
    assert_eq!(
        alice.get_balance().await?,
        beyond_the_pool,
        "a refused swap leaves the user's funds where they were"
    );

    Ok(())
}

struct PoolUser {
    wallet: SparkWallet,
    signer: Arc<dyn Signer>,
    config: SparkWalletConfig,
}

async fn add_pool_user(
    fixtures: &TestFixtures,
    ssp_config: &ServiceProviderConfig,
    deposit_sats: u64,
) -> Result<PoolUser> {
    let config = fixtures
        .create_wallet_config_with_ssp(Some(ssp_config.clone()))
        .await?;
    let signer: Arc<dyn Signer> = Arc::new(create_test_signer_bob());
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let wallet = SparkWallet::connect(config.clone(), spark_signer).await?;

    let mut events = wallet.subscribe_events();
    wallet.start_background_processing().await;
    loop {
        if matches!(events.recv().await?, WalletEvent::Synced) {
            break;
        }
    }
    deposit_with_amount(&wallet, &fixtures.bitcoind, deposit_sats).await?;

    Ok(PoolUser {
        wallet,
        signer,
        config,
    })
}

#[tokio::test]
#[test_log::test]
async fn test_swap_never_hands_one_leaf_to_two_users() -> Result<()> {
    const USERS: usize = 4;

    let fixture = setup_swap_fixture().await?;
    let top_denomination = 1u64 << spark_itest::fixtures::sspd::MAX_DENOMINATION_POWER;
    // Asked of the daemon, since trees are built whole and the pool can hold more
    // than its target.
    let stocked = *fixture
        .fixtures
        .sspd()
        .await?
        .pool_leaf_counts()
        .await?
        .get(&top_denomination)
        .context("the pool stocks the top denomination")?;

    let leaves_wanted = (stocked as usize / USERS) + 1;
    let want = vec![top_denomination; leaves_wanted];
    let deposit = top_denomination * leaves_wanted as u64;
    info!(
        "{USERS} users want {leaves_wanted} leaves of {top_denomination} each; the pool holds {stocked}"
    );

    let mut users = Vec::with_capacity(USERS);
    for _ in 0..USERS {
        users.push(add_pool_user(&fixture.fixtures, &fixture.ssp_config, deposit).await?);
    }

    // Concurrent, since sequential swaps would each see the pool the previous one
    // left.
    let ssp_config = fixture.ssp_config.clone();
    let swaps = users.iter().map(|user| {
        let ssp_config = ssp_config.clone();
        let want = want.clone();
        async move {
            let leaves = user.wallet.list_leaves().await?;
            let leaf = leaves
                .available
                .first()
                .context("a funded user has a leaf to swap")?
                .id
                .clone();
            client_swap_leaves(
                &user.config,
                user.signer.clone(),
                &ssp_config,
                &[leaf],
                Some(want),
            )
            .await
        }
    });
    let outcomes = futures::future::join_all(swaps).await;

    let mut received: Vec<TreeNodeId> = Vec::new();
    let mut served = 0usize;
    let mut refused = 0usize;
    for (index, outcome) in outcomes.iter().enumerate() {
        match outcome {
            Ok(nodes) => {
                served += 1;
                received.extend(nodes.iter().map(|node| node.id.clone()));
            }
            Err(e) => {
                refused += 1;
                info!("user {index} was refused, as some had to be: {e:?}");
            }
        }
    }
    info!("{served} users served, {refused} refused");

    let mut seen = std::collections::HashSet::new();
    for leaf in &received {
        assert!(
            seen.insert(leaf.clone()),
            "leaf {leaf} was handed to two users at once"
        );
    }

    let mut given = std::collections::HashSet::new();
    for swap in fixture.swaps().await? {
        for leaf in &swap.outbound {
            assert!(
                given.insert(leaf.leaf_id.clone()),
                "the daemon recorded leaf {} as sent in two swaps",
                leaf.leaf_id
            );
        }
    }

    assert!(
        served > 0,
        "the pool could serve someone, so someone must have been served"
    );
    assert!(
        refused > 0,
        "{USERS} users wanting {leaves_wanted} leaves each of a denomination the pool holds \
         {stocked} of must not all be served"
    );

    Ok(())
}

fn pool_ceiling_sats() -> u64 {
    let per_denomination = u64::from(spark_itest::fixtures::sspd::LEAVES_PER_DENOMINATION);
    (0..=spark_itest::fixtures::sspd::MAX_DENOMINATION_POWER)
        .map(|power| per_denomination * (1u64 << power))
        .sum()
}

#[tokio::test]
#[test_log::test]
async fn test_swap_refronts_a_reclaimed_pool_leaf() -> Result<()> {
    const ROUNDS: usize = 6;

    let fixture = setup_swap_fixture().await?;
    let alice = &fixture.alice_wallet;
    let bitcoind = &fixture.fixtures.bitcoind;

    deposit_with_amount(alice, bitcoind, 3072).await?;

    for round in 0..ROUNDS {
        let held: Vec<TreeNodeId> = alice
            .list_leaves()
            .await?
            .available
            .iter()
            .map(|leaf| leaf.id.clone())
            .collect();
        client_swap_leaves(
            &fixture.alice_config,
            fixture.alice_signer.clone(),
            &fixture.ssp_config,
            &held,
            None,
        )
        .await
        .with_context(|| format!("swap {round} failed"))?;
        alice.sync().await?;

        if let Some(leaf_id) = refronted_leaf(&fixture.swaps().await?) {
            info!("daemon re-fronted reclaimed leaf {leaf_id} by swap {round}");
            return Ok(());
        }
        // Gives the asynchronous claim of this swap's leaves time to land before
        // the next swap.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    anyhow::bail!(
        "the daemon never re-fronted a leaf it had claimed back across {ROUNDS} swaps, so \
         whether it can still sign one was not exercised"
    )
}

/// `swaps` is newest first, so it is walked in reverse to check each swap against
/// only earlier ones.
fn refronted_leaf(swaps: &[internal_api::Swap]) -> Option<String> {
    let mut reclaimed: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for swap in swaps.iter().rev() {
        if let Some(leaf) = swap
            .outbound
            .iter()
            .find(|leaf| reclaimed.contains(leaf.leaf_id.as_str()))
        {
            return Some(leaf.leaf_id.clone());
        }
        reclaimed.extend(swap.inbound.iter().map(|leaf| leaf.leaf_id.as_str()));
    }
    None
}
