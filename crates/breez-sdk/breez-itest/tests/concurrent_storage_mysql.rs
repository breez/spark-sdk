//! Multi-instance concurrent storage integration tests against MySQL.
//!
//! Mirrors `concurrent_storage.rs`, swapping the testcontainers backend from
//! `Postgres` to `Mysql` and using `build_sdk_with_mysql`. Validates that
//! multiple SDK instances (same wallet/seed) connecting to the same MySQL
//! database behave correctly under concurrent load with actual payments.

use std::collections::HashSet;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mysql::Mysql;
use tracing::info;

/// Test fixture for concurrent multi-instance tests, MySQL-backed.
struct MysqlConcurrentTestFixture {
    /// MySQL container - must be kept alive for the test duration.
    #[allow(dead_code)]
    mysql_container: ContainerAsync<Mysql>,
    /// Connection string for the MySQL database.
    connection_string: String,
    /// Shared seed used by all main wallet instances.
    shared_seed: [u8; 32],
}

impl MysqlConcurrentTestFixture {
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

/// Multi-instance concurrent operations test against MySQL.
///
/// Validates that two SDK instances backed by the same MySQL database can:
/// - Sync concurrently without deadlocking
/// - See the same payment list and balance after operations
/// - Handle a concurrent send-while-syncing scenario
///
/// This mirrors the postgres `test_concurrent_multi_instance_operations` but
/// uses a smaller scenario count to keep CI time reasonable. Full coverage of
/// the entire breez-itest suite against MySQL is available via
/// `make breez-itest-mysql-tree-store`.
#[test_log::test(tokio::test)]
async fn test_mysql_concurrent_multi_instance_operations() -> Result<()> {
    info!("=== Starting test_mysql_concurrent_multi_instance_operations ===");

    info!("Setting up test fixture with MySQL container...");
    let fixture = MysqlConcurrentTestFixture::new().await?;

    info!("Creating first SDK instance and funding...");
    let mut instance_0 = fixture.build_instance().await?;

    info!("Funding main wallet via faucet...");
    ensure_funded(&mut instance_0, 4_000).await?;

    info!("Creating second SDK instance with shared seed...");
    let instance_1 = fixture.build_instance().await?;

    info!("Creating counterparty SDK with SQLite storage...");
    let counterparty_dir = tempdir::TempDir::new("breez-sdk-counterparty")?;
    let counterparty_path = counterparty_dir.path().to_string_lossy().to_string();
    let mut counterparty_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut counterparty_seed);
    let mut counterparty =
        build_sdk_with_dir(counterparty_path, counterparty_seed, Some(counterparty_dir)).await?;

    let mut expected_payment_count: usize = 1; // initial deposit
    let initial_balance = instance_0
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!(
        "Initial state: {} payments, {} sats balance",
        expected_payment_count, initial_balance
    );

    // Scenario 1: concurrent sync.
    info!("=== Scenario 1: Concurrent sync ===");
    let (sync_0, sync_1) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
    );
    sync_0?;
    sync_1?;

    let payments_0 = instance_0
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let payments_1 = instance_1
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    assert_eq!(payments_0.payments.len(), expected_payment_count);
    assert_eq!(payments_1.payments.len(), expected_payment_count);

    // Scenario 2: send + concurrent sync.
    info!("=== Scenario 2: Send + concurrent sync ===");
    let counterparty_address = counterparty
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let payment_amount = 1000u64;
    let prepare = instance_0
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: counterparty_address.clone(),
            amount: Some(payment_amount.into()),
            token_identifier: None,
            fee_policy: None,
            conversion_options: None,
        })
        .await?;

    let (send_result, sync_1_result) = tokio::join!(
        instance_0.sdk.send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        }),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
    );
    send_result?;
    sync_1_result?;

    wait_for_payment_succeeded_event(&mut counterparty.events, PaymentType::Receive, 60).await?;
    expected_payment_count += 1;

    let (_, _) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
    );

    let payments_0 = instance_0
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let payments_1 = instance_1
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let ids_0: HashSet<_> = payments_0.payments.iter().map(|p| &p.id).collect();
    let ids_1: HashSet<_> = payments_1.payments.iter().map(|p| &p.id).collect();
    assert_eq!(payments_0.payments.len(), expected_payment_count);
    assert_eq!(ids_0, ids_1, "instance 0 and 1 should see same payment IDs");

    let balance_after_send = instance_0
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    assert_eq!(balance_after_send, initial_balance - payment_amount);

    info!("=== test_mysql_concurrent_multi_instance_operations PASSED ===");
    Ok(())
}
