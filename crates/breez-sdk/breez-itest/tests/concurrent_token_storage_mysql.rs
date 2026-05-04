//! Concurrent token storage stress test against MySQL.
//!
//! Mirrors `concurrent_token_storage.rs`, swapping the testcontainers backend
//! from `Postgres` to `Mysql` and using `build_sdk_with_mysql`. Validates that
//! multiple SDK instances backed by the same MySQL database handle token
//! operations correctly under concurrent load.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mysql::Mysql;
use tracing::info;

/// Test fixture for concurrent token tests with MySQL backend.
struct MysqlConcurrentTokenFixture {
    #[allow(dead_code)]
    mysql_container: ContainerAsync<Mysql>,
    connection_string: String,
    shared_seed: [u8; 32],
}

impl MysqlConcurrentTokenFixture {
    async fn new() -> Result<Self> {
        let mysql_container = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");

        let host_port = mysql_container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");

        let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

        let mut shared_seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut shared_seed);

        Ok(Self {
            mysql_container,
            connection_string,
            shared_seed,
        })
    }

    async fn build_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_mysql(&self.connection_string, self.shared_seed).await
    }
}

async fn get_token_balance(sdk: &BreezSdk, token_identifier: &str) -> Result<u128> {
    let info = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    Ok(info
        .token_balances
        .get(token_identifier)
        .map(|b| b.balance)
        .unwrap_or(0))
}

/// Multi-instance concurrent token operations test against MySQL.
///
/// Verifies that two MySQL-backed SDK instances see consistent token state
/// after concurrent syncs and a token transfer. Smaller scope than the
/// postgres equivalent to keep CI time reasonable. Full coverage runs via
/// `make breez-itest-mysql-tree-store`.
#[test_log::test(tokio::test)]
async fn test_mysql_concurrent_token_operations() -> Result<()> {
    info!("=== Starting test_mysql_concurrent_token_operations ===");

    info!("Setting up test fixture with MySQL container...");
    let fixture = MysqlConcurrentTokenFixture::new().await?;

    info!("Creating SDK instances with shared seed...");
    let instance_0 = fixture.build_instance().await?;
    let instance_1 = fixture.build_instance().await?;

    info!("Creating and minting test token...");
    let issuer = instance_0.sdk.get_token_issuer();
    let token_metadata = issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "mysql-test-tkn".to_string(),
            ticker: "MTT".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply: 1_000_000,
        })
        .await?;

    issuer
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    instance_0.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let token_id = token_metadata.identifier.clone();
    info!("Token created: {} ({})", token_metadata.name, token_id);

    let alice_initial = get_token_balance(&instance_0.sdk, &token_id).await?;
    assert_eq!(alice_initial, 1_000_000);

    info!("Creating Bob with SQLite storage...");
    let bob_dir = tempdir::TempDir::new("breez-sdk-bob-tokens-mysql")?;
    let bob_path = bob_dir.path().to_string_lossy().to_string();
    let mut bob_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bob_seed);
    let bob = build_sdk_with_dir(bob_path, bob_seed, Some(bob_dir)).await?;

    info!("Sending 250,000 tokens to Bob...");
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = instance_0
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_address,
            amount: Some(250_000),
            token_identifier: Some(token_id.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    instance_0
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    wait_for_token_balance_increase(&bob.sdk, &token_id, 0, 60).await?;

    // Concurrent sync verification — both instances must converge on the same balance.
    let (sync_0, sync_1) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
    );
    sync_0?;
    sync_1?;

    let bal_0 = get_token_balance(&instance_0.sdk, &token_id).await?;
    let bal_1 = get_token_balance(&instance_1.sdk, &token_id).await?;
    assert_eq!(bal_0, 750_000);
    assert_eq!(
        bal_1, 750_000,
        "Instance 1 should see the same token balance"
    );

    info!("=== test_mysql_concurrent_token_operations PASSED ===");
    Ok(())
}
