//! Backend-agnostic scenarios for multi-instance concurrent storage tests.
//!
//! The postgres and mysql variants of `concurrent_storage` /
//! `concurrent_token_storage` share the same wallet workflow with a
//! pluggable backend factory. Centralising the bodies here gives every
//! backend identical coverage automatically.
//!
//! Each scenario takes a `build_instance` closure that produces a fresh
//! `SdkInstance` bound to the shared backend (e.g. one DB shared across
//! instances). Backend-specific tests provide it via their fixture and
//! retain the testcontainer for the scenario's lifetime.
//!
//! Usage from a test file:
//!
//! ```ignore
//! let fixture = MyFixture::new().await?;
//! run_concurrent_multi_instance_operations(|| fixture.build_instance()).await
//! ```
//!
//! `Fn() -> Future` is required (called multiple times). The fixture must
//! own the testcontainer + seed and produce fresh instances on each call.

use std::collections::HashSet;
use std::future::Future;

use anyhow::Result;
use breez_sdk_spark::*;
use rand::RngCore;
use tracing::info;

use crate::SdkInstance;
use crate::helpers::*;

/// Token-amount per-payment for the bidirectional stress loop.
const PAYMENT_AMOUNT: u128 = 100;
/// Token payments sent in each direction per batch (Alice→Bob then Bob→Alice).
const PAYMENTS_PER_DIRECTION: usize = 5;
/// Bidirectional batches in the token stress loop.
const NUM_BATCHES: usize = 3;

/// Comprehensive multi-instance concurrent operations test (BTC).
///
/// Validates that three SDK instances sharing the same backend correctly handle:
/// - Concurrent wallet syncing
/// - Payment send while other instances sync
/// - Concurrent balance/payment queries while a payment is received
/// - Stress loop of rotating sends with concurrent reads
///
/// `build_instance` is invoked four times (3 main wallet instances + an
/// optional warm path); each call must yield a fresh SDK bound to the same
/// shared backend with the same seed.
#[allow(clippy::too_many_lines)]
pub async fn run_concurrent_multi_instance_operations<F, Fut>(build_instance: F) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<SdkInstance>>,
{
    info!("Creating first SDK instance and funding...");
    let mut instance_0 = build_instance().await?;

    // Fund the main wallet via faucet (need ~2500 sats: 1000 + 100×5 stress).
    info!("Funding main wallet via faucet...");
    ensure_funded(&mut instance_0, 4_000).await?;

    // Now create instances 1 and 2 (after funding to avoid claim race).
    info!("Creating additional SDK instances with shared seed...");
    let instance_1 = build_instance().await?;
    let instance_2 = build_instance().await?;

    // Counterparty with a different seed using SQLite (standard setup).
    info!("Creating counterparty SDK with SQLite storage...");
    let counterparty_dir = tempfile::Builder::new()
        .prefix("breez-sdk-counterparty")
        .tempdir()?;
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

    // Scenario 1: Concurrent sync.
    info!("=== Scenario 1: Concurrent sync ===");
    let (sync_0, sync_1, sync_2) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );
    sync_0?;
    sync_1?;
    sync_2?;

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

    assert_eq!(payments_0.payments.len(), expected_payment_count);
    assert_eq!(payments_1.payments.len(), expected_payment_count);
    assert_eq!(payments_2.payments.len(), expected_payment_count);
    info!(
        "Scenario 1 passed: All instances see {} payments",
        payments_0.payments.len()
    );

    // Scenario 2: Send + concurrent sync.
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

    wait_for_payment_succeeded_event(&mut counterparty.events, PaymentType::Receive, 60).await?;
    expected_payment_count += 1;

    let (_, _, _) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );

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

    assert_eq!(payments_0.payments.len(), expected_payment_count);
    assert_eq!(ids_0, ids_1, "Instance 0 and 1 should see same payment IDs");
    assert_eq!(ids_1, ids_2, "Instance 1 and 2 should see same payment IDs");
    assert_eq!(
        payments_0.payments.len(),
        ids_0.len(),
        "No duplicate payment IDs on instance 0"
    );

    let balance_after_send = instance_0
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    let expected_balance_after_send = initial_balance - payment_amount;
    assert_eq!(balance_after_send, expected_balance_after_send);

    info!(
        "Scenario 2 passed: {} payments, balance {} -> {} sats",
        payments_0.payments.len(),
        initial_balance,
        balance_after_send
    );

    // Scenario 3: Receive + concurrent query.
    info!("=== Scenario 3: Receive + concurrent query ===");
    let main_address = instance_1
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let return_amount = 500u64;
    let prepare_return = counterparty
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: main_address,
            amount: Some(return_amount.into()),
            token_identifier: None,
            fee_policy: None,
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
    expected_payment_count += 1;

    let expected_balance_after_receive = expected_balance_after_send + return_amount;

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
    assert_eq!(balance_0, expected_balance_after_receive);
    assert_eq!(balance_1, expected_balance_after_receive);
    assert_eq!(balance_2, expected_balance_after_receive);

    info!(
        "Scenario 3 passed: All instances show balance {} sats",
        balance_0
    );

    // Scenario 4: Stress loop — 5 rounds of rotating payments with concurrent sync.
    info!("=== Scenario 4: Stress loop (5 rounds) ===");
    let instances = [instance_0, instance_1, instance_2];
    let mut current_balance = expected_balance_after_receive;

    for round in 0..5 {
        let sender_idx = round % 3;
        let payment_amt = 100u64;

        info!(
            "Round {}: Instance {} sends {} sats",
            round, sender_idx, payment_amt
        );

        let cp_address = counterparty
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::SparkAddress,
            })
            .await?
            .payment_request;

        let prepare = instances[sender_idx]
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: cp_address,
                amount: Some(payment_amt.into()),
                token_identifier: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

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

        wait_for_payment_succeeded_event(&mut counterparty.events, PaymentType::Receive, 60)
            .await?;
        expected_payment_count += 1;
        current_balance -= payment_amt;

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

    // Final verification.
    info!("=== Final verification ===");
    let (_, _, _) = tokio::join!(
        instances[0].sdk.sync_wallet(SyncWalletRequest {}),
        instances[1].sdk.sync_wallet(SyncWalletRequest {}),
        instances[2].sdk.sync_wallet(SyncWalletRequest {})
    );

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

    assert_eq!(final_payments_0.payments.len(), expected_payment_count);
    assert_eq!(final_payments_1.payments.len(), expected_payment_count);
    assert_eq!(final_payments_2.payments.len(), expected_payment_count);

    let all_ids: HashSet<_> = final_payments_0.payments.iter().map(|p| &p.id).collect();
    assert_eq!(
        all_ids.len(),
        expected_payment_count,
        "No duplicate payment IDs"
    );

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

    assert_eq!(info_0?.balance_sats, current_balance);
    assert_eq!(info_1?.balance_sats, current_balance);
    assert_eq!(info_2?.balance_sats, current_balance);

    info!(
        "=== Test PASSED: {} payments, {} sats, no duplicates, no deadlocks ===",
        final_payments_0.payments.len(),
        current_balance
    );
    Ok(())
}

/// Stress test for concurrent token operations across three instances.
///
/// Two sides exchange `NUM_BATCHES * PAYMENTS_PER_DIRECTION * 2` token payments
/// while multiple instances of the issuer sync against the same shared
/// backend.
#[allow(clippy::too_many_lines)]
pub async fn run_concurrent_token_operations<F, Fut>(build_instance: F) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<SdkInstance>>,
{
    info!("Creating SDK instances with shared seed...");
    let instance_0 = build_instance().await?;
    let instance_1 = build_instance().await?;
    let instance_2 = build_instance().await?;

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
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    instance_0.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let token_id = token_metadata.identifier.clone();
    info!("Token created: {} ({})", token_metadata.name, token_id);

    let alice_initial = get_token_balance(&instance_0.sdk, &token_id).await?;
    assert_eq!(alice_initial, 1_000_000);

    info!("Creating Bob with SQLite storage...");
    let bob_dir = tempfile::Builder::new()
        .prefix("breez-sdk-bob-tokens")
        .tempdir()?;
    let bob_path = bob_dir.path().to_string_lossy().to_string();
    let mut bob_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bob_seed);
    let bob = build_sdk_with_dir(bob_path, bob_seed, Some(bob_dir)).await?;

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

    wait_for_token_balance_increase(&bob.sdk, &token_id, 0, 60).await?;

    instance_0.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let mut alice_token_balance = get_token_balance(&instance_0.sdk, &token_id).await?;
    let mut bob_token_balance = get_token_balance(&bob.sdk, &token_id).await?;
    assert_eq!(alice_token_balance, 500_000);
    assert_eq!(bob_token_balance, 500_000);

    let mut expected_token_payment_count = get_token_payment_count(&instance_0.sdk).await?;
    info!(
        "Initial token payment count: {}",
        expected_token_payment_count
    );

    // Phase 1: Concurrent sync verification.
    info!("=== Phase 1: Concurrent sync verification ===");
    let (sync_0, sync_1, sync_2) = tokio::join!(
        instance_0.sdk.sync_wallet(SyncWalletRequest {}),
        instance_1.sdk.sync_wallet(SyncWalletRequest {}),
        instance_2.sdk.sync_wallet(SyncWalletRequest {})
    );
    sync_0?;
    sync_1?;
    sync_2?;

    assert_eq!(
        get_token_balance(&instance_0.sdk, &token_id).await?,
        alice_token_balance
    );
    assert_eq!(
        get_token_balance(&instance_1.sdk, &token_id).await?,
        alice_token_balance
    );
    assert_eq!(
        get_token_balance(&instance_2.sdk, &token_id).await?,
        alice_token_balance
    );
    assert_eq!(
        get_token_payment_count(&instance_0.sdk).await?,
        expected_token_payment_count
    );
    assert_eq!(
        get_token_payment_count(&instance_1.sdk).await?,
        expected_token_payment_count
    );
    assert_eq!(
        get_token_payment_count(&instance_2.sdk).await?,
        expected_token_payment_count
    );
    info!("Phase 1 passed: All instances consistent");

    // Phase 2: Stress loop.
    info!(
        "=== Phase 2: Stress loop ({} batches × {} bidirectional payments) ===",
        NUM_BATCHES,
        PAYMENTS_PER_DIRECTION * 2
    );

    let instances = [instance_0, instance_1, instance_2];

    for batch in 0..NUM_BATCHES {
        info!("--- Batch {}/{} ---", batch + 1, NUM_BATCHES);

        // Alice → Bob direction: rotate sender, others sync.
        for i in 0..PAYMENTS_PER_DIRECTION {
            let sender_idx = i % 3;

            let bob_addr = bob
                .sdk
                .receive_payment(ReceivePaymentRequest {
                    payment_method: ReceivePaymentMethod::SparkAddress,
                })
                .await?
                .payment_request;

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

            let syncer_idxs: Vec<usize> = (0..3).filter(|&j| j != sender_idx).collect();
            match sender_idx {
                0 => {
                    let (send, s1, s2) = tokio::join!(
                        instances[0].sdk.send_payment(SendPaymentRequest {
                            prepare_response: prepare,
                            options: None,
                            idempotency_key: None,
                        }),
                        instances[syncer_idxs[0]]
                            .sdk
                            .sync_wallet(SyncWalletRequest {}),
                        instances[syncer_idxs[1]]
                            .sdk
                            .sync_wallet(SyncWalletRequest {})
                    );
                    send?;
                    s1?;
                    s2?;
                }
                1 => {
                    let (s0, send, s2) = tokio::join!(
                        instances[syncer_idxs[0]]
                            .sdk
                            .sync_wallet(SyncWalletRequest {}),
                        instances[1].sdk.send_payment(SendPaymentRequest {
                            prepare_response: prepare,
                            options: None,
                            idempotency_key: None,
                        }),
                        instances[syncer_idxs[1]]
                            .sdk
                            .sync_wallet(SyncWalletRequest {})
                    );
                    s0?;
                    send?;
                    s2?;
                }
                2 => {
                    let (s0, s1, send) = tokio::join!(
                        instances[syncer_idxs[0]]
                            .sdk
                            .sync_wallet(SyncWalletRequest {}),
                        instances[syncer_idxs[1]]
                            .sdk
                            .sync_wallet(SyncWalletRequest {}),
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

        wait_for_token_balance(&bob.sdk, &token_id, bob_token_balance, 120).await?;
        info!(
            "Batch {} Alice→Bob complete: Alice={}, Bob={}",
            batch + 1,
            alice_token_balance,
            bob_token_balance
        );

        // Bob → Alice direction: Bob sends, all 3 Alice instances sync concurrently.
        for i in 0..PAYMENTS_PER_DIRECTION {
            let receiver_idx = i % 3;
            let alice_addr = instances[receiver_idx]
                .sdk
                .receive_payment(ReceivePaymentRequest {
                    payment_method: ReceivePaymentMethod::SparkAddress,
                })
                .await?
                .payment_request;

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

        wait_for_token_balance(&instances[0].sdk, &token_id, alice_token_balance, 120).await?;
        info!(
            "Batch {} Bob→Alice complete: Alice={}, Bob={}",
            batch + 1,
            alice_token_balance,
            bob_token_balance
        );
    }

    // Phase 3: Final verification.
    info!("=== Phase 3: Final verification ===");
    let (s0, s1, s2) = tokio::join!(
        instances[0].sdk.sync_wallet(SyncWalletRequest {}),
        instances[1].sdk.sync_wallet(SyncWalletRequest {}),
        instances[2].sdk.sync_wallet(SyncWalletRequest {})
    );
    s0?;
    s1?;
    s2?;

    assert_eq!(
        get_token_balance(&instances[0].sdk, &token_id).await?,
        alice_token_balance
    );
    assert_eq!(
        get_token_balance(&instances[1].sdk, &token_id).await?,
        alice_token_balance
    );
    assert_eq!(
        get_token_balance(&instances[2].sdk, &token_id).await?,
        alice_token_balance
    );

    let final_count_0 = get_token_payment_count(&instances[0].sdk).await?;
    let final_count_1 = get_token_payment_count(&instances[1].sdk).await?;
    let final_count_2 = get_token_payment_count(&instances[2].sdk).await?;
    assert_eq!(final_count_0, expected_token_payment_count);
    assert_eq!(final_count_1, expected_token_payment_count);
    assert_eq!(final_count_2, expected_token_payment_count);

    let ids_0 = get_token_payment_ids(&instances[0].sdk).await?;
    let ids_1 = get_token_payment_ids(&instances[1].sdk).await?;
    let ids_2 = get_token_payment_ids(&instances[2].sdk).await?;
    assert_eq!(ids_0, ids_1);
    assert_eq!(ids_1, ids_2);
    assert_eq!(ids_0.len(), expected_token_payment_count);

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final = get_token_balance(&bob.sdk, &token_id).await?;
    assert_eq!(bob_final, bob_token_balance);

    info!(
        "=== Test PASSED: {} token payments, Alice={}, Bob={}, no duplicates, no deadlocks ===",
        expected_token_payment_count, alice_token_balance, bob_token_balance
    );
    Ok(())
}

// ----- private token helpers used only by this module ---------------------

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
