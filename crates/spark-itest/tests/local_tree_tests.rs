use std::sync::Arc;

use anyhow::Result;
use bitcoin::Amount;
use rstest::*;
use spark::operator::OperatorPool;
use spark::operator::rpc::{ConnectionManager, DefaultConnectionManager};
use spark::session_store::InMemorySessionStore;
use spark::signer::{DefaultSigner, Signer, SparkSigner, SparkSignerAdapter};
use spark::tree::TreeNodeId;
use spark_itest::helpers::{WalletsFixture, wallets};
use sspd_lib::tree::builder::build_tree_blueprint;
use sspd_lib::tree::deposit::TreeDepositService;
use tracing::info;

async fn deposit_and_create_tree(
    fixture: &WalletsFixture,
    leaf_values: Vec<u64>,
) -> Result<Vec<u64>> {
    let bitcoind = &fixture.fixtures.bitcoind;
    let total_sats: u64 = leaf_values.iter().sum();

    let seed = [3u8; 32];
    let network = spark::Network::Regtest;
    let signer: Arc<dyn Signer> = Arc::new(DefaultSigner::new(&seed, network)?);
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let identity_public_key = spark::signer::derive_identity_public_key(signer.as_ref()).await?;

    let wallet_config = fixture.fixtures.create_wallet_config().await?;
    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn ConnectionManager> = Arc::new(DefaultConnectionManager::new());
    let operator_pool = Arc::new(
        OperatorPool::connect(
            &wallet_config.operator_pool,
            connection_manager,
            session_store,
            spark_signer.clone(),
            None,
        )
        .await?,
    );

    let tree_deposit_service =
        TreeDepositService::new(identity_public_key, network, operator_pool.clone(), signer);

    let blueprint = build_tree_blueprint(leaf_values, 2)?;
    let plan = tree_deposit_service.plan_deposit_tree(&blueprint).await?;

    let bitcoin_service = spark::bitcoin::BitcoinService::new(network);
    let signer2: Arc<dyn Signer> = Arc::new(DefaultSigner::new(&seed, network)?);
    let spark_signer2: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer2.clone()));
    let session_store2 = Arc::new(InMemorySessionStore::default());
    let service_provider = Arc::new(spark::ssp::ServiceProvider::new(
        spark::ssp::ServiceProviderConfig {
            base_url: String::new(),
            schema_endpoint: None,
            identity_public_key,
            user_agent: None,
            retry_config: spark::ssp::RetryConfig::default(),
        },
        spark_signer2.clone(),
        session_store2,
        None,
    )?);
    let deposit_service = spark::services::DepositService::new(
        bitcoin_service,
        identity_public_key,
        network,
        operator_pool,
        service_provider,
        spark_signer2,
    );

    let root_id = TreeNodeId::generate();
    let deposit_address = deposit_service
        .generate_deposit_address(plan.root_public_key, &root_id)
        .await?;
    let address = deposit_address.address;

    let txid = bitcoind
        .fund_address(&address, Amount::from_sat(total_sats))
        .await?;

    bitcoind.generate_blocks(3).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 3).await?;

    fixture
        .fixtures
        .spark_so
        .wait_for_log("tree not found in available or creating status")
        .await?;

    let tx = bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .enumerate()
        .find_map(|(i, output)| {
            bitcoin::Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
                .ok()
                .filter(|a| *a == address)
                .map(|_| i as u32)
        })
        .expect("deposit output not found");

    let n_leaves = plan.leaf_ids.len();

    let created = tree_deposit_service
        .execute_deposit_tree(
            &blueprint,
            &plan.leaf_ids,
            &deposit_address.verifying_public_key,
            tx,
            vout,
        )
        .await?;

    info!("Created tree: {} leaves", created.pairs.len());

    assert_eq!(
        created.pairs.len(),
        n_leaves,
        "expected {n_leaves} leaves, got {}",
        created.pairs.len()
    );

    let leaf_sum: u64 = created.pairs.iter().map(|(l, _)| l.value).sum();
    assert_eq!(leaf_sum, total_sats, "leaf values must sum to deposit");

    let mut values: Vec<u64> = created.pairs.iter().map(|(l, _)| l.value).collect();
    values.sort_unstable();
    Ok(values)
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_tree_2_leaves(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let leaves = deposit_and_create_tree(&fixture, vec![1024, 2048]).await?;
    assert_eq!(leaves, vec![1024, 2048]);
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_tree_3_leaves(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let leaves = deposit_and_create_tree(&fixture, vec![1024, 2048, 4096]).await?;
    assert_eq!(leaves, vec![1024, 2048, 4096]);
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_tree_4_leaves(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let leaves = deposit_and_create_tree(&fixture, vec![1024; 4]).await?;
    assert_eq!(leaves.len(), 4);
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_tree_5_leaves(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let leaves = deposit_and_create_tree(&fixture, vec![1024; 5]).await?;
    assert_eq!(leaves.len(), 5);
    Ok(())
}

#[rstest]
#[tokio::test]
#[test_log::test]
async fn test_tree_mixed_denominations(#[future] wallets: WalletsFixture) -> Result<()> {
    let fixture = wallets.await;
    let leaves = deposit_and_create_tree(&fixture, vec![4096, 2048, 1024, 1024, 512, 512]).await?;
    assert_eq!(leaves.len(), 6);
    let total: u64 = leaves.iter().sum();
    assert_eq!(total, 9216);
    Ok(())
}
