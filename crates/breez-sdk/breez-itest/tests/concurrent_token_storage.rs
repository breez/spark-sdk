//! Concurrent token storage stress test.
//!
//! Validates that multiple SDK instances (same wallet/seed) connecting to the same
//! PostgreSQL database handle token operations correctly under concurrent load
//! with actual bidirectional token payments.
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
//!       │ issuer      │ │ syncer     │ │ syncer     │
//!       └──────┬──────┘ └─────┬──────┘ └─────┬──────┘
//!              │              │              │
//!              └──────────────┼──────────────┘
//!                             │ token payments (bidirectional)
//!                       ┌─────▼──────┐
//!                       │    Bob     │
//!                       │ (seed B)   │
//!                       │ SQLite     │
//!                       └────────────┘
//! ```

use std::collections::HashSet;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tracing::info;

const PAYMENT_AMOUNT: u128 = 100;
const PAYMENTS_PER_DIRECTION: usize = 5;
const NUM_BATCHES: usize = 3;

/// Test fixture for concurrent token tests with PostgreSQL backend
struct ConcurrentTestFixture {
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
    connection_string: String,
    shared_seed: [u8; 32],
}

impl ConcurrentTestFixture {
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

        let mut shared_seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut shared_seed);

        Ok(Self {
            pg_container,
            connection_string,
            shared_seed,
        })
    }

    async fn build_postgres_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_postgres(&self.connection_string, self.shared_seed).await
    }
}

/// Helper to get token balance from an SDK instance
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

/// Helper to count token payments from list_payments
async fn get_token_payment_count(sdk: &BreezSdk) -> Result<usize> {
    let payments = sdk
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(AssetFilter::Token {
                token_identifier: None,
            }),
            ..Default::default()
        })
        .await?;
    Ok(payments.payments.len())
}

/// Helper to get token payment IDs
async fn get_token_payment_ids(sdk: &BreezSdk) -> Result<HashSet<String>> {
    let payments = sdk
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(AssetFilter::Token {
                token_identifier: None,
            }),
            ..Default::default()
        })
        .await?;
    Ok(payments.payments.into_iter().map(|p| p.id).collect())
}

/// Stress test for concurrent token operations across multiple PostgreSQL-backed instances.
///
/// Two sides send 100 token payments back and forth (50 each direction) using
/// batched concurrent sends while multiple instances sync against the same database.
#[test_log::test(tokio::test)]
async fn test_concurrent_token_operations() -> Result<()> {
    info!("=== Starting test_concurrent_token_operations ===");

    // --- Setup phase ---
    info!("Setting up test fixture with PostgreSQL container...");
    let fixture = ConcurrentTestFixture::new().await?;

    // Create first instance and fund with sats (needed for token operations)
    info!("Creating instance_0 and funding with sats...");
    let mut instance_0 = fixture.build_postgres_instance().await?;
    ensure_funded(&mut instance_0, 4_000).await?;

    // Create and mint token via instance_0
    info!("Creating and minting test token...");
    let issuer = instance_0.sdk.get_token_issuer();
    let token_metadata = issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "concurrent-test-tkn".to_string(),
            ticker: "CTT".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply: 1_000_000,
        })
        .await?;

    issuer
        .mint_issuer_token(MintIssuerTokenRequest {
            amount: 1_000_000,
        })
        .await?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    instance_0.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let token_id = token_metadata.identifier.clone();
    info!(
        "Token created: {} ({})",
        token_metadata.name, token_id
    );

    // Verify Alice has all tokens
    let alice_initial = get_token_balance(&instance_0.sdk, &token_id).await?;
    assert_eq!(alice_initial, 1_000_000, "Alice should have 1,000,000 tokens after minting");

    // Now create instances 1 and 2 (after funding + minting to avoid claim race)
    info!("Creating additional SDK instances with shared seed...");
    let instance_1 = fixture.build_postgres_instance().await?;
    let instance_2 = fixture.build_postgres_instance().await?;

    // Create Bob (SQLite, different seed) and fund with sats
    info!("Creating Bob with SQLite storage...");
    let bob_dir = tempdir::TempDir::new("breez-sdk-bob-tokens")?;
    let bob_path = bob_dir.path().to_string_lossy().to_string();
    let mut bob_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bob_seed);
    let mut bob = build_sdk_with_dir(bob_path, bob_seed, Some(bob_dir)).await?;
    ensure_funded(&mut bob, 4_000).await?;

    // Send initial tokens to Bob (500,000) so both sides have tokens
    info!("Sending 500,000 tokens to Bob...");
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
            amount: Some(500_000),
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

    // Wait for Bob to receive tokens
    wait_for_token_balance_increase(&bob.sdk, &token_id, 0, 60).await?;

    // Record initial state
    instance_0.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let mut alice_token_balance = get_token_balance(&instance_0.sdk, &token_id).await?;
    let mut bob_token_balance = get_token_balance(&bob.sdk, &token_id).await?;

    info!(
        "Initial token state: Alice={}, Bob={}",
        alice_token_balance, bob_token_balance
    );
    assert_eq!(alice_token_balance, 500_000, "Alice should have 500,000 tokens");
    assert_eq!(bob_token_balance, 500_000, "Bob should have 500,000 tokens");

    // Count initial token payments (the initial transfer from Alice to Bob)
    let mut expected_token_payment_count = get_token_payment_count(&instance_0.sdk).await?;
    info!(
        "Initial token payment count: {}",
        expected_token_payment_count
    );

    // --- Phase 1: Concurrent sync verification ---
    info!("=== Phase 1: Concurrent sync verification ===");
    let (sync_0, sync_1, sync_2) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );
    sync_0?;
    sync_1?;
    sync_2?;

    // Verify all instances see same token balance
    let bal_0 = get_token_balance(&instance_0.sdk, &token_id).await?;
    let bal_1 = get_token_balance(&instance_1.sdk, &token_id).await?;
    let bal_2 = get_token_balance(&instance_2.sdk, &token_id).await?;

    assert_eq!(bal_0, alice_token_balance, "Instance 0 token balance mismatch");
    assert_eq!(bal_1, alice_token_balance, "Instance 1 token balance mismatch");
    assert_eq!(bal_2, alice_token_balance, "Instance 2 token balance mismatch");

    let count_0 = get_token_payment_count(&instance_0.sdk).await?;
    let count_1 = get_token_payment_count(&instance_1.sdk).await?;
    let count_2 = get_token_payment_count(&instance_2.sdk).await?;

    assert_eq!(count_0, expected_token_payment_count, "Instance 0 payment count mismatch");
    assert_eq!(count_1, expected_token_payment_count, "Instance 1 payment count mismatch");
    assert_eq!(count_2, expected_token_payment_count, "Instance 2 payment count mismatch");
    info!("Phase 1 passed: All instances consistent");

    // --- Phase 2: Stress loop ---
    info!("=== Phase 2: Stress loop ({} batches of {} bidirectional payments) ===", NUM_BATCHES, PAYMENTS_PER_DIRECTION * 2);

    let instances = [instance_0, instance_1, instance_2];

    for batch in 0..NUM_BATCHES {
        info!("--- Batch {}/{} ---", batch + 1, NUM_BATCHES);

        // --- Alice → Bob direction: send PAYMENTS_PER_DIRECTION payments sequentially ---
        // Rotate sender across instances, other instances sync concurrently
        for i in 0..PAYMENTS_PER_DIRECTION {
            let sender_idx = i % 3;

            // Get Bob's address
            let bob_addr = bob
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
                    payment_request: bob_addr,
                    amount: Some(PAYMENT_AMOUNT),
                    token_identifier: Some(token_id.clone()),
                    conversion_options: None,
                    fee_policy: None,
                })
                .await?;

            // Send from sender while other instances sync concurrently
            let syncer_idxs: Vec<usize> = (0..3).filter(|&j| j != sender_idx).collect();
            match sender_idx {
                0 => {
                    let (send, s1, s2) = tokio::join!(
                        instances[0].sdk.send_payment(SendPaymentRequest {
                            prepare_response: prepare,
                            options: None,
                            idempotency_key: None,
                        }),
                        instances[syncer_idxs[0]].sdk.sync_wallet(SyncWalletRequest {}),
                        instances[syncer_idxs[1]].sdk.sync_wallet(SyncWalletRequest {})
                    );
                    send?;
                    s1?;
                    s2?;
                }
                1 => {
                    let (s0, send, s2) = tokio::join!(
                        instances[syncer_idxs[0]].sdk.sync_wallet(SyncWalletRequest {}),
                        instances[1].sdk.send_payment(SendPaymentRequest {
                            prepare_response: prepare,
                            options: None,
                            idempotency_key: None,
                        }),
                        instances[syncer_idxs[1]].sdk.sync_wallet(SyncWalletRequest {})
                    );
                    s0?;
                    send?;
                    s2?;
                }
                2 => {
                    let (s0, s1, send) = tokio::join!(
                        instances[syncer_idxs[0]].sdk.sync_wallet(SyncWalletRequest {}),
                        instances[syncer_idxs[1]].sdk.sync_wallet(SyncWalletRequest {}),
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
        }

        alice_token_balance -= PAYMENTS_PER_DIRECTION as u128 * PAYMENT_AMOUNT;
        bob_token_balance += PAYMENTS_PER_DIRECTION as u128 * PAYMENT_AMOUNT;
        expected_token_payment_count += PAYMENTS_PER_DIRECTION;

        // Wait for Bob to reach expected balance
        wait_for_token_balance(&bob.sdk, &token_id, bob_token_balance, 120).await?;
        info!(
            "Batch {} Alice→Bob complete: Alice={}, Bob={}",
            batch + 1,
            alice_token_balance,
            bob_token_balance
        );

        // --- Bob → Alice direction: Bob sends PAYMENTS_PER_DIRECTION payments sequentially ---
        // All 3 Alice instances sync concurrently via background tasks
        for i in 0..PAYMENTS_PER_DIRECTION {
            // Get Alice's address from a rotating instance
            let receiver_idx = i % 3;
            let alice_addr = instances[receiver_idx]
                .sdk
                .receive_payment(ReceivePaymentRequest {
                    payment_method: ReceivePaymentMethod::SparkAddress,
                })
                .await?
                .payment_request;

            // Bob prepares and sends
            let prepare = bob
                .sdk
                .prepare_send_payment(PrepareSendPaymentRequest {
                    payment_request: alice_addr,
                    amount: Some(PAYMENT_AMOUNT),
                    token_identifier: Some(token_id.clone()),
                    conversion_options: None,
                    fee_policy: None,
                })
                .await?;

            // Bob sends while all Alice instances sync concurrently
            let (send, s0, s1, s2) = tokio::join!(
                bob.sdk.send_payment(SendPaymentRequest {
                    prepare_response: prepare,
                    options: None,
                    idempotency_key: None,
                }),
                instances[0].sdk.sync_wallet(SyncWalletRequest {}),
                instances[1].sdk.sync_wallet(SyncWalletRequest {}),
                instances[2].sdk.sync_wallet(SyncWalletRequest {})
            );
            send?;
            s0?;
            s1?;
            s2?;
        }

        bob_token_balance -= PAYMENTS_PER_DIRECTION as u128 * PAYMENT_AMOUNT;
        alice_token_balance += PAYMENTS_PER_DIRECTION as u128 * PAYMENT_AMOUNT;
        expected_token_payment_count += PAYMENTS_PER_DIRECTION;

        // Wait for Alice instance_0 to reach expected balance
        wait_for_token_balance(&instances[0].sdk, &token_id, alice_token_balance, 120).await?;
        info!(
            "Batch {} Bob→Alice complete: Alice={}, Bob={}",
            batch + 1,
            alice_token_balance,
            bob_token_balance
        );

        info!(
            "Batch {}/{} complete: {} total token payments expected",
            batch + 1,
            NUM_BATCHES,
            expected_token_payment_count
        );
    }

    // --- Phase 3: Final verification ---
    info!("=== Phase 3: Final verification ===");

    // Sync all instances one final time
    let (s0, s1, s2) = tokio::join!(
        instances[0].sdk.sync_wallet(SyncWalletRequest {}),
        instances[1].sdk.sync_wallet(SyncWalletRequest {}),
        instances[2].sdk.sync_wallet(SyncWalletRequest {})
    );
    s0?;
    s1?;
    s2?;

    // Verify token balances on all Alice instances
    let final_bal_0 = get_token_balance(&instances[0].sdk, &token_id).await?;
    let final_bal_1 = get_token_balance(&instances[1].sdk, &token_id).await?;
    let final_bal_2 = get_token_balance(&instances[2].sdk, &token_id).await?;

    assert_eq!(
        final_bal_0, alice_token_balance,
        "Instance 0 final token balance should be {}",
        alice_token_balance
    );
    assert_eq!(
        final_bal_1, alice_token_balance,
        "Instance 1 final token balance should match"
    );
    assert_eq!(
        final_bal_2, alice_token_balance,
        "Instance 2 final token balance should match"
    );

    // Verify token payment counts
    let final_count_0 = get_token_payment_count(&instances[0].sdk).await?;
    let final_count_1 = get_token_payment_count(&instances[1].sdk).await?;
    let final_count_2 = get_token_payment_count(&instances[2].sdk).await?;

    assert_eq!(
        final_count_0, expected_token_payment_count,
        "Instance 0 should have exactly {} token payments",
        expected_token_payment_count
    );
    assert_eq!(
        final_count_1, expected_token_payment_count,
        "Instance 1 should have exactly {} token payments",
        expected_token_payment_count
    );
    assert_eq!(
        final_count_2, expected_token_payment_count,
        "Instance 2 should have exactly {} token payments",
        expected_token_payment_count
    );

    // Verify payment IDs are identical across all instances and no duplicates
    let ids_0 = get_token_payment_ids(&instances[0].sdk).await?;
    let ids_1 = get_token_payment_ids(&instances[1].sdk).await?;
    let ids_2 = get_token_payment_ids(&instances[2].sdk).await?;

    assert_eq!(ids_0, ids_1, "Instance 0 and 1 should see same token payment IDs");
    assert_eq!(ids_1, ids_2, "Instance 1 and 2 should see same token payment IDs");
    assert_eq!(
        ids_0.len(),
        expected_token_payment_count,
        "No duplicate token payment IDs (found {} unique, expected {})",
        ids_0.len(),
        expected_token_payment_count
    );

    // Verify Bob's final token balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final = get_token_balance(&bob.sdk, &token_id).await?;
    assert_eq!(
        bob_final, bob_token_balance,
        "Bob's final token balance should be {}",
        bob_token_balance
    );

    info!(
        "=== Test PASSED: {} token payments, Alice balance={} (expected {}), Bob balance={} (expected {}), no duplicates, no deadlocks ===",
        expected_token_payment_count,
        final_bal_0,
        alice_token_balance,
        bob_final,
        bob_token_balance
    );

    Ok(())
}
