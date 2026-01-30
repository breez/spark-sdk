//! Multi-instance concurrent storage integration tests.
//!
//! Validates that multiple SDK instances (same wallet/seed) connecting to the same
//! PostgreSQL database (each with own connection pool) behave correctly under
//! concurrent load with actual payments.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                   PostgreSQL Container                   │
//! │                    (testcontainers)                      │
//! └────────────┬──────────────┬──────────────┬──────────────┘
//!              │              │              │
//!       ┌──────▼──────┐ ┌─────▼──────┐ ┌─────▼──────┐
//!       │ Instance 0  │ │ Instance 1 │ │ Instance 2 │
//!       │ (seed A)    │ │ (seed A)   │ │ (seed A)   │
//!       │ own pool    │ │ own pool   │ │ own pool   │
//!       └──────┬──────┘ └─────┬──────┘ └─────┬──────┘
//!              │              │              │
//!              └──────────────┼──────────────┘
//!                             │ Spark transfers
//!                       ┌─────▼──────┐
//!                       │ Counterparty│
//!                       │ (seed B)    │
//!                       │ SQLite      │
//!                       └─────────────┘
//! ```

use std::collections::HashSet;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tracing::info;

/// Test fixture for concurrent multi-instance tests
struct ConcurrentTestFixture {
    /// PostgreSQL container - must be kept alive for the test duration
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
    /// Connection string for the PostgreSQL database
    connection_string: String,
    /// Shared seed used by all main wallet instances
    shared_seed: [u8; 32],
}

impl ConcurrentTestFixture {
    /// Creates a new test fixture with a PostgreSQL container
    async fn new() -> Result<Self> {
        let pg_container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");

        let host_port = pg_container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");

        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );

        // Generate random seed for the shared wallet
        let mut shared_seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut shared_seed);

        Ok(Self {
            pg_container,
            connection_string,
            shared_seed,
        })
    }

    /// Builds an SDK instance connected to the shared PostgreSQL database
    async fn build_postgres_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_postgres(&self.connection_string, self.shared_seed).await
    }
}

/// Comprehensive test for concurrent multi-instance operations.
///
/// This test validates that multiple SDK instances sharing the same PostgreSQL
/// database correctly handle concurrent operations including:
/// - Concurrent wallet syncing
/// - Payment operations during concurrent syncs
/// - Concurrent balance and payment queries
/// - Stress testing with multiple payment rounds
#[test_log::test(tokio::test)]
async fn test_concurrent_multi_instance_operations() -> Result<()> {
    info!("=== Starting test_concurrent_multi_instance_operations ===");

    // Setup: Start PostgreSQL container and create instances
    info!("Setting up test fixture with PostgreSQL container...");
    let fixture = ConcurrentTestFixture::new().await?;

    // Create first SDK instance and fund it BEFORE creating other instances
    // This avoids race conditions where another instance might claim the deposit
    info!("Creating first SDK instance and funding...");
    let mut instance_0 = fixture.build_postgres_instance().await?;

    // Fund the main wallet via faucet (only need enough for test payments)
    // We need ~2500 sats total: 1000 (scenario 2) + 100 × 5 rounds (scenario 4)
    info!("Funding main wallet via faucet...");
    ensure_funded(&mut instance_0, 4_000).await?;

    // Now create instances 1 and 2 (after funding to avoid claim race)
    info!("Creating additional SDK instances with shared seed...");
    let instance_1 = fixture.build_postgres_instance().await?;
    let instance_2 = fixture.build_postgres_instance().await?;

    // Create counterparty with different seed using SQLite (standard setup)
    info!("Creating counterparty SDK with SQLite storage...");
    let counterparty_dir = tempdir::TempDir::new("breez-sdk-counterparty")?;
    let counterparty_path = counterparty_dir.path().to_string_lossy().to_string();
    let mut counterparty_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut counterparty_seed);
    let mut counterparty =
        build_sdk_with_dir(counterparty_path, counterparty_seed, Some(counterparty_dir)).await?;

    // Track expected state
    let mut expected_payment_count: usize = 1; // Initial deposit from funding
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

    // Scenario 1: Concurrent sync - all 3 instances sync simultaneously
    info!("=== Scenario 1: Concurrent sync ===");
    let (sync_0, sync_1, sync_2) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );
    sync_0?;
    sync_1?;
    sync_2?;

    // Verify all instances see the same state AND match expected
    let payments_0 = instance_0
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let payments_1 = instance_1
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let payments_2 = instance_2
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;

    assert_eq!(
        payments_0.payments.len(),
        expected_payment_count,
        "Instance 0 should have {} payments",
        expected_payment_count
    );
    assert_eq!(
        payments_1.payments.len(),
        expected_payment_count,
        "Instance 1 should have {} payments",
        expected_payment_count
    );
    assert_eq!(
        payments_2.payments.len(),
        expected_payment_count,
        "Instance 2 should have {} payments",
        expected_payment_count
    );
    info!(
        "Scenario 1 passed: All instances see {} payments (expected: {})",
        payments_0.payments.len(),
        expected_payment_count
    );

    // Scenario 2: Send payment while other instances sync
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
            pay_amount: Some(PayAmount::Bitcoin {
                amount_sats: payment_amount,
            }),
            conversion_options: None,
        })
        .await?;

    // Start syncs on instances 1 and 2 concurrently with the payment
    let (send_result, sync_1_result, sync_2_result) = tokio::join!(
        instance_0.sdk.send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        }),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );

    send_result?;
    sync_1_result?;
    sync_2_result?;

    // Wait for payment to complete on counterparty
    wait_for_payment_succeeded_event(&mut counterparty.events, PaymentType::Receive, 60).await?;
    expected_payment_count += 1; // The send payment

    // Sync all instances to ensure they see the payment
    let (_, _, _) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );

    // Verify payment visible on all instances with no duplicates
    let payments_0 = instance_0
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let payments_1 = instance_1
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let payments_2 = instance_2
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;

    let ids_0: HashSet<_> = payments_0.payments.iter().map(|p| &p.id).collect();
    let ids_1: HashSet<_> = payments_1.payments.iter().map(|p| &p.id).collect();
    let ids_2: HashSet<_> = payments_2.payments.iter().map(|p| &p.id).collect();

    assert_eq!(
        payments_0.payments.len(),
        expected_payment_count,
        "Instance 0 should have {} payments after send",
        expected_payment_count
    );
    assert_eq!(ids_0, ids_1, "Instance 0 and 1 should see same payment IDs");
    assert_eq!(ids_1, ids_2, "Instance 1 and 2 should see same payment IDs");
    assert_eq!(
        payments_0.payments.len(),
        ids_0.len(),
        "No duplicate payment IDs on instance 0"
    );

    // Verify balance decreased by payment amount
    let balance_after_send = instance_0
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    let expected_balance_after_send = initial_balance - payment_amount;
    assert_eq!(
        balance_after_send, expected_balance_after_send,
        "Balance should decrease by {} sats",
        payment_amount
    );

    info!(
        "Scenario 2 passed: {} payments, balance {} -> {} sats",
        payments_0.payments.len(),
        initial_balance,
        balance_after_send
    );

    // Scenario 3: Receive payment while all instances query concurrently
    info!("=== Scenario 3: Receive + concurrent query ===");

    // Get main wallet's Spark address from instance 1
    let main_address = instance_1
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Counterparty received 1000 sats in Scenario 2, send some back
    let return_amount = 500u64;
    let prepare_return = counterparty
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: main_address,
            pay_amount: Some(PayAmount::Bitcoin {
                amount_sats: return_amount,
            }),
            conversion_options: None,
        })
        .await?;

    counterparty
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_return,
            options: None,
            idempotency_key: None,
        })
        .await?;
    expected_payment_count += 1; // The receive payment

    // Expected balance: initial - sent + received
    let expected_balance_after_receive = expected_balance_after_send + return_amount;

    // Wait for the payment to be received by waiting for the balance to update.
    // This is more reliable than waiting for events since multiple instances share the wallet.
    wait_for_balance(
        &instance_0.sdk,
        Some(expected_balance_after_receive),
        None,
        60,
    )
    .await?;

    let (info_0, info_1, info_2) = tokio::join!(
        instance_0.sdk.get_info(GetInfoRequest {
            ensure_synced: Some(true)
        }),
        instance_1.sdk.get_info(GetInfoRequest {
            ensure_synced: Some(true)
        }),
        instance_2.sdk.get_info(GetInfoRequest {
            ensure_synced: Some(true)
        })
    );

    let balance_0 = info_0?.balance_sats;
    let balance_1 = info_1?.balance_sats;
    let balance_2 = info_2?.balance_sats;

    assert_eq!(
        balance_0, expected_balance_after_receive,
        "Instance 0 balance should be {} sats",
        expected_balance_after_receive
    );
    assert_eq!(
        balance_1, expected_balance_after_receive,
        "Instance 1 balance should match"
    );
    assert_eq!(
        balance_2, expected_balance_after_receive,
        "Instance 2 balance should match"
    );

    info!(
        "Scenario 3 passed: All instances show balance {} sats (expected: {})",
        balance_0, expected_balance_after_receive
    );

    // Scenario 4: Stress loop - 5 rounds of rotating payments with concurrent operations
    info!("=== Scenario 4: Stress loop (5 rounds) ===");
    let instances = [instance_0, instance_1, instance_2];

    // Track balance through stress loop
    let mut current_balance = expected_balance_after_receive;

    for round in 0..5 {
        let sender_idx = round % 3;
        let payment_amt = 100u64;

        info!(
            "Round {}: Instance {} sends {} sats",
            round, sender_idx, payment_amt
        );

        // Get counterparty address
        let cp_address = counterparty
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::SparkAddress,
            })
            .await?
            .payment_request;

        // Prepare on sender instance
        let prepare = instances[sender_idx]
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: cp_address,
                pay_amount: Some(PayAmount::Bitcoin {
                    amount_sats: payment_amt,
                }),
                conversion_options: None,
            })
            .await?;

        // Send from sender, sync from others concurrently
        match sender_idx {
            0 => {
                let (send, s1, s2) = tokio::join!(
                    instances[0].sdk.send_payment(SendPaymentRequest {
                        prepare_response: prepare,
                        options: None,
                        idempotency_key: None,
                    }),
                    instances[1].sdk.sync_wallet(SyncWalletRequest {}),
                    instances[2].sdk.sync_wallet(SyncWalletRequest {})
                );
                send?;
                s1?;
                s2?;
            }
            1 => {
                let (s0, send, s2) = tokio::join!(
                    instances[0].sdk.sync_wallet(SyncWalletRequest {}),
                    instances[1].sdk.send_payment(SendPaymentRequest {
                        prepare_response: prepare,
                        options: None,
                        idempotency_key: None,
                    }),
                    instances[2].sdk.sync_wallet(SyncWalletRequest {})
                );
                s0?;
                send?;
                s2?;
            }
            2 => {
                let (s0, s1, send) = tokio::join!(
                    instances[0].sdk.sync_wallet(SyncWalletRequest {}),
                    instances[1].sdk.sync_wallet(SyncWalletRequest {}),
                    instances[2].sdk.send_payment(SendPaymentRequest {
                        prepare_response: prepare,
                        options: None,
                        idempotency_key: None,
                    })
                );
                s0?;
                s1?;
                send?;
            }
            _ => unreachable!(),
        }

        // Wait for counterparty to receive
        wait_for_payment_succeeded_event(&mut counterparty.events, PaymentType::Receive, 60)
            .await?;
        expected_payment_count += 1;
        current_balance -= payment_amt;

        // Sync all and verify
        let (_, _, _) = tokio::join!(
            instances[0].sdk.sync_wallet(SyncWalletRequest {}),
            instances[1].sdk.sync_wallet(SyncWalletRequest {}),
            instances[2].sdk.sync_wallet(SyncWalletRequest {})
        );

        info!(
            "Round {} complete: expected {} payments, {} sats balance",
            round, expected_payment_count, current_balance
        );
    }

    // Final verification
    info!("=== Final verification ===");

    // Sync all instances one final time
    let (_, _, _) = tokio::join!(
        instances[0].sdk.sync_wallet(SyncWalletRequest {}),
        instances[1].sdk.sync_wallet(SyncWalletRequest {}),
        instances[2].sdk.sync_wallet(SyncWalletRequest {})
    );

    // Check payment counts
    let final_payments_0 = instances[0]
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let final_payments_1 = instances[1]
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;
    let final_payments_2 = instances[2]
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?;

    // Verify expected payment count
    assert_eq!(
        final_payments_0.payments.len(),
        expected_payment_count,
        "Instance 0 should have exactly {} payments",
        expected_payment_count
    );
    assert_eq!(
        final_payments_1.payments.len(),
        expected_payment_count,
        "Instance 1 should have exactly {} payments",
        expected_payment_count
    );
    assert_eq!(
        final_payments_2.payments.len(),
        expected_payment_count,
        "Instance 2 should have exactly {} payments",
        expected_payment_count
    );

    // Verify no duplicate IDs
    let all_ids: HashSet<_> = final_payments_0.payments.iter().map(|p| &p.id).collect();
    assert_eq!(
        all_ids.len(),
        expected_payment_count,
        "No duplicate payment IDs (found {} unique, expected {})",
        all_ids.len(),
        expected_payment_count
    );

    // Check balances match expected
    let (info_0, info_1, info_2) = tokio::join!(
        instances[0].sdk.get_info(GetInfoRequest {
            ensure_synced: Some(false)
        }),
        instances[1].sdk.get_info(GetInfoRequest {
            ensure_synced: Some(false)
        }),
        instances[2].sdk.get_info(GetInfoRequest {
            ensure_synced: Some(false)
        })
    );

    let balance_0 = info_0?.balance_sats;
    let balance_1 = info_1?.balance_sats;
    let balance_2 = info_2?.balance_sats;

    assert_eq!(
        balance_0, current_balance,
        "Instance 0 final balance should be {} sats",
        current_balance
    );
    assert_eq!(
        balance_1, current_balance,
        "Instance 1 final balance should match"
    );
    assert_eq!(
        balance_2, current_balance,
        "Instance 2 final balance should match"
    );

    info!(
        "=== Test PASSED: {} payments (expected {}), {} sats balance (expected {}), no duplicates, no deadlocks ===",
        final_payments_0.payments.len(),
        expected_payment_count,
        balance_0,
        current_balance
    );

    Ok(())
}
