//! Wallet recovery integration tests
//!
//! These tests verify that a wallet can be correctly recovered from a mnemonic
//! and that all historical payments are properly synced from Spark operators.
//!
//! # Setup Test
//! The `test_setup_recovery_wallet` test (marked `#[ignore]`) creates a new wallet
//! with all payment variants and outputs the mnemonic and expected payments JSON.
//! Run it manually with: `cargo test test_setup_recovery_wallet -- --ignored --nocapture`
//!
//! # Recovery Test
//! The `test_wallet_recovery_from_mnemonic` test loads a wallet from mnemonic
//! (via environment variables) and verifies all payments match the expected spec.
//! It skips automatically if the environment variables are not set.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use serde::{Deserialize, Serialize};
use tempdir::TempDir;
use tracing::{info, warn};

// ============================================================================
// Expected Payment Structures
// ============================================================================

/// Expected payments specification for recovery testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedRecoveryPayments {
    /// Expected balance after recovery
    pub balance_sats: u64,
    /// Expected payments to verify
    pub payments: Vec<ExpectedPayment>,
}

/// Single expected payment specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedPayment {
    /// Payment ID: `<transaction_hash>:<output_index>`
    pub id: String,
    /// "send" or "receive"
    pub payment_type: String,
    /// "lightning", "spark", "token", "deposit", "withdraw"
    pub method: String,
    /// Expected amount (sats or token base units)
    pub amount: u128,
    /// Expected status: "completed", "pending", "failed"
    pub status: String,
    /// Expected fees (0 for receives, calculated for sends)
    pub fees: u128,
    /// Exact timestamp (unix seconds) - captured after payment is final
    pub timestamp: u64,
    /// Variant-specific details to assert
    #[serde(flatten)]
    pub details: Option<ExpectedPaymentDetails>,
}

/// Variant-specific payment details for assertions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "details_type")]
pub enum ExpectedPaymentDetails {
    /// For Spark HTLC payments
    SparkHtlc {
        payment_hash: String,
        preimage: Option<String>,
    },
    /// For Lightning payments
    Lightning {
        payment_hash: String,
        preimage: Option<String>,
    },
    /// For Deposit/Withdraw
    OnChain { tx_id: String },
    /// For Token payments
    Token { token_identifier: String },
}

// ============================================================================
// Configuration
// ============================================================================

/// Recovery test configuration loaded from environment
struct RecoveryTestConfig {
    mnemonic: String,
    expected: ExpectedRecoveryPayments,
}

/// Load recovery test configuration from environment variables
///
/// Returns None if either RECOVERY_TEST_MNEMONIC or RECOVERY_TEST_EXPECTED_PAYMENTS
/// is not set, allowing the test to skip gracefully.
fn recovery_test_config() -> Option<RecoveryTestConfig> {
    let mnemonic = std::env::var("RECOVERY_TEST_MNEMONIC").ok()?;
    let expected_json = std::env::var("RECOVERY_TEST_EXPECTED_PAYMENTS").ok()?;

    let expected: ExpectedRecoveryPayments = serde_json::from_str(&expected_json)
        .map_err(|e| {
            warn!("Failed to parse RECOVERY_TEST_EXPECTED_PAYMENTS: {}", e);
            e
        })
        .ok()?;

    Some(RecoveryTestConfig { mnemonic, expected })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Build expected payments JSON from actual payments
///
/// This captures all payment details after they are finalized, ensuring
/// timestamps and other details are stable for assertion.
fn build_expected_payments(payments: &[Payment], balance_sats: u64) -> ExpectedRecoveryPayments {
    let expected_payments: Vec<ExpectedPayment> = payments
        .iter()
        .map(|p| {
            let details = match &p.details {
                Some(PaymentDetails::Spark {
                    htlc_details: Some(htlc),
                    ..
                }) => Some(ExpectedPaymentDetails::SparkHtlc {
                    payment_hash: htlc.payment_hash.clone(),
                    preimage: htlc.preimage.clone(),
                }),
                Some(PaymentDetails::Lightning { htlc_details, .. }) => {
                    Some(ExpectedPaymentDetails::Lightning {
                        payment_hash: htlc_details.payment_hash.clone(),
                        preimage: htlc_details.preimage.clone(),
                    })
                }
                Some(PaymentDetails::Deposit { tx_id }) => Some(ExpectedPaymentDetails::OnChain {
                    tx_id: tx_id.clone(),
                }),
                Some(PaymentDetails::Withdraw { tx_id }) => Some(ExpectedPaymentDetails::OnChain {
                    tx_id: tx_id.clone(),
                }),
                Some(PaymentDetails::Token { metadata, .. }) => {
                    Some(ExpectedPaymentDetails::Token {
                        token_identifier: metadata.identifier.clone(),
                    })
                }
                _ => None,
            };

            ExpectedPayment {
                id: p.id.clone(),
                payment_type: p.payment_type.to_string(),
                method: p.method.to_string(),
                amount: p.amount,
                status: p.status.to_string(),
                fees: p.fees,
                timestamp: p.timestamp,
                details,
            }
        })
        .collect();

    ExpectedRecoveryPayments {
        balance_sats,
        payments: expected_payments,
    }
}

/// Create and mint a test token for the setup test
async fn create_mint_test_token(instance: &SdkInstance) -> Result<TokenMetadata> {
    let issuer = instance.sdk.get_token_issuer();
    let token_metadata = issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "Recovery Test Token".to_string(),
            ticker: "RTT".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply: 100_000_000,
        })
        .await?;

    issuer
        .mint_issuer_token(MintIssuerTokenRequest {
            amount: 100_000_000,
        })
        .await?;

    info!("Minted 100,000,000 tokens ({})", token_metadata.identifier);

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    Ok(token_metadata)
}

// ============================================================================
// Setup Test
// ============================================================================

/// Setup test: Create a wallet with all payment variants
///
/// Run manually with: `cargo test test_setup_recovery_wallet -- --ignored --nocapture`
///
/// This test:
/// 1. Generates a new mnemonic for Alice (primary wallet)
/// 2. Creates Bob (helper wallet) for bidirectional payments
/// 3. Creates all payment variants (deposit, spark, lightning, token, withdraw)
/// 4. Outputs the mnemonic and expected payments JSON
///
/// After running, copy the output and set as GitHub secrets:
/// - RECOVERY_TEST_MNEMONIC
/// - RECOVERY_TEST_EXPECTED_PAYMENTS
#[rstest]
#[ignore] // Only run manually
#[test_log::test(tokio::test)]
async fn test_setup_recovery_wallet() -> Result<()> {
    info!("=== Starting test_setup_recovery_wallet ===");
    info!("This test creates a wallet with all payment variants for recovery testing.");

    // 1. Generate new mnemonic for Alice (primary wallet)
    // Generate 16 bytes of entropy for a 12-word mnemonic
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    let alice_mnemonic = bip39::Mnemonic::from_entropy(&entropy)?;
    info!("Generated mnemonic for Alice");

    let alice_temp_dir = TempDir::new("breez-sdk-alice")?;
    let mut alice = build_sdk_from_mnemonic(
        alice_temp_dir.path().to_string_lossy().to_string(),
        alice_mnemonic.to_string(),
        None,
        Some(alice_temp_dir),
    )
    .await?;

    // 2. Create Bob (helper wallet) with random seed
    let mut bob = alice_sdk().await?;

    // 3. Fund Alice via faucet → Deposit (receive)
    // Note: We only fund Alice - Bob gets funds from Alice's payments
    info!("Funding Alice via faucet (creates Deposit payment)...");
    receive_and_fund(&mut alice, 50_000, true).await?;

    // 4. Spark regular: Alice → Bob (send) - gives Bob funds for subsequent payments
    info!("Creating Spark payment: Alice → Bob...");
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(10_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    // 5. Spark regular: Bob → Alice (receive)
    info!("Creating Spark payment: Bob → Alice...");
    let alice_spark_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            amount: Some(3_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

    // 6. Spark HTLC: Alice → Bob (send with HTLC)
    info!("Creating Spark HTLC payment: Alice → Bob...");
    let (preimage1, payment_hash1) = generate_preimage_hash_pair();

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(2_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::SparkAddress {
                htlc_options: Some(SparkHtlcOptions {
                    payment_hash: payment_hash1.clone(),
                    expiry_duration_secs: 180,
                }),
            }),
            idempotency_key: None,
        })
        .await?;

    // Bob claims the HTLC
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

    bob.sdk
        .claim_htlc_payment(ClaimHtlcPaymentRequest {
            preimage: preimage1,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    // 7. Spark HTLC: Bob → Alice (receive with HTLC)
    info!("Creating Spark HTLC payment: Bob → Alice...");
    let (preimage2, payment_hash2) = generate_preimage_hash_pair();

    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            amount: Some(1_500),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::SparkAddress {
                htlc_options: Some(SparkHtlcOptions {
                    payment_hash: payment_hash2.clone(),
                    expiry_duration_secs: 180,
                }),
            }),
            idempotency_key: None,
        })
        .await?;

    // Alice claims the HTLC
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    alice
        .sdk
        .claim_htlc_payment(ClaimHtlcPaymentRequest {
            preimage: preimage2,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

    // 8. Lightning: Alice → Bob (send, prefer_spark: false)
    info!("Creating Lightning payment: Alice → Bob...");
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Recovery test lightning payment".to_string(),
                amount_sats: Some(1_000),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice,
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(30),
            }),
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    // 9. Lightning: Bob → Alice (receive)
    info!("Creating Lightning payment: Bob → Alice...");
    let alice_invoice = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Recovery test lightning receive".to_string(),
                amount_sats: Some(800),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_invoice,
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(30),
            }),
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

    // 10. Token: Create token on Alice, transfer to Bob, Bob sends back
    info!("Creating Token payments...");
    let token_metadata = create_mint_test_token(&alice).await?;

    // Alice sends tokens to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address.clone(),
            amount: Some(100),
            token_identifier: Some(token_metadata.identifier.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

    // Bob sends tokens back to Alice
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_spark_address.clone(),
            amount: Some(50),
            token_identifier: Some(token_metadata.identifier.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

    // 10a. Create many token payments to test pagination during sync
    // Sync fetches token transactions in pages of 50 (PAYMENT_SYNC_BATCH_SIZE)
    // We create 60 total token payments (30 each direction) to ensure pagination is triggered
    info!("Creating 60 token payments for pagination testing...");
    const TOKEN_PAYMENT_PAIRS: u32 = 30;

    for i in 0..TOKEN_PAYMENT_PAIRS {
        // Alice → Bob
        let prepare = alice
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: bob_spark_address.clone(),
                amount: Some(1000),
                token_identifier: Some(token_metadata.identifier.clone()),
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

        alice
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await?;

        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

        // Bob → Alice
        bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

        let prepare = bob
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: alice_spark_address.clone(),
                amount: Some(1000),
                token_identifier: Some(token_metadata.identifier.clone()),
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

        bob.sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await?;

        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Receive, 60).await?;

        if (i + 1) % 10 == 0 {
            info!("Completed {} token payment pairs", i + 1);
        }
    }

    info!("Completed all 60 token payments for pagination testing");

    // 11. Withdraw: Alice withdraws on-chain
    info!("Creating Withdraw payment...");

    // Get Bob's deposit address as the withdrawal destination
    let withdraw_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: withdraw_address,
            amount: Some(10_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for withdraw to be broadcast (initial event)
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 120).await?;

    // 12. Wait for withdraw to be confirmed on-chain (requires block confirmations)
    info!("Waiting for withdraw to be confirmed on-chain...");
    let withdraw_timeout = std::time::Duration::from_secs(300); // 5 minutes max
    let poll_interval = std::time::Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let payments = alice
            .sdk
            .list_payments(ListPaymentsRequest::default())
            .await?
            .payments;

        // Find the withdraw payment
        let withdraw = payments
            .iter()
            .find(|p| p.method == PaymentMethod::Withdraw);

        if let Some(withdraw) = withdraw {
            if withdraw.status == PaymentStatus::Completed {
                info!("Withdraw confirmed!");
                break;
            }
            info!(
                "Withdraw status: {:?}, waiting for confirmation...",
                withdraw.status
            );
        }

        if start.elapsed() > withdraw_timeout {
            panic!(
                "Withdraw confirmation timeout after {} seconds - test failed. Last status: {:?}",
                withdraw_timeout.as_secs(),
                withdraw.map(|w| &w.status)
            );
        }

        tokio::time::sleep(poll_interval).await;
    }

    // 13. Final sync to ensure all payments are captured
    info!("Final sync...");
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    // Get final balance and payments
    let info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    let payments = alice
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?
        .payments;

    // Verify all payments are completed
    for payment in &payments {
        if payment.status != PaymentStatus::Completed {
            warn!(
                "Payment {} is not completed yet: {:?}",
                payment.id, payment.status
            );
        }
    }

    // 14. Build expected payments JSON from FINALIZED payments
    let expected = build_expected_payments(&payments, info.balance_sats);

    info!("\n\n");
    info!("=== RECOVERY TEST WALLET CREATED ===");
    info!("Final balance: {} sats", info.balance_sats);
    info!("Total payments: {}", payments.len());
    info!("\n");
    info!("=== MNEMONIC (add as RECOVERY_TEST_MNEMONIC secret) ===");
    info!("{}", alice_mnemonic);
    info!("\n");
    info!("=== EXPECTED PAYMENTS JSON (add as RECOVERY_TEST_EXPECTED_PAYMENTS secret) ===");
    info!("{}", serde_json::to_string(&expected)?);
    info!("\n");
    info!("=== EXPECTED PAYMENTS JSON (pretty) ===");
    info!("{}", serde_json::to_string_pretty(&expected)?);
    info!("\n\n");

    info!("=== Test test_setup_recovery_wallet COMPLETED ===");
    Ok(())
}

// ============================================================================
// Recovery Test
// ============================================================================

/// Test wallet recovery from mnemonic
///
/// This test verifies that a wallet can be fully recovered from its mnemonic
/// phrase by syncing with Spark operators and retrieving all historical payments.
///
/// Requires environment variables:
/// - RECOVERY_TEST_MNEMONIC: BIP-39 mnemonic phrase
/// - RECOVERY_TEST_EXPECTED_PAYMENTS: JSON with expected payment data
///
/// If either variable is not set, the test skips gracefully.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_wallet_recovery_from_mnemonic() -> Result<()> {
    info!("=== Starting test_wallet_recovery_from_mnemonic ===");

    // 1. Load config from env, skip if not present
    let Some(config) = recovery_test_config() else {
        // In CI, secrets should be configured - fail if missing
        if std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok() {
            panic!(
                "Recovery test secrets not configured in CI! \
                 Set RECOVERY_TEST_MNEMONIC and RECOVERY_TEST_EXPECTED_PAYMENTS"
            );
        }
        // Locally, skip gracefully
        warn!(
            "Skipping test_wallet_recovery_from_mnemonic: \
             RECOVERY_TEST_MNEMONIC or RECOVERY_TEST_EXPECTED_PAYMENTS not set"
        );
        return Ok(());
    };

    info!(
        "Loaded recovery test config: {} expected payments",
        config.expected.payments.len()
    );

    // 2. Create fresh storage (simulates new device)
    let temp_dir = TempDir::new("breez-sdk-recovery")?;
    let storage_path = temp_dir.path().to_string_lossy().to_string();

    info!("Initializing wallet from mnemonic at: {}", storage_path);

    // 3. Initialize SDK from mnemonic
    let sdk_instance =
        build_sdk_from_mnemonic(storage_path, config.mnemonic, None, Some(temp_dir)).await?;

    // 4. Wait for sync
    info!("Waiting for wallet sync...");
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;

    // 5. Verify balance
    let info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    info!("Recovered balance: {} sats", info.balance_sats);

    assert_eq!(
        info.balance_sats, config.expected.balance_sats,
        "Balance {} != expected {}",
        info.balance_sats, config.expected.balance_sats
    );

    // 6. List payments
    let payments = sdk_instance
        .sdk
        .list_payments(ListPaymentsRequest::default())
        .await?
        .payments;

    info!("Recovered {} payments", payments.len());

    assert_eq!(
        payments.len(),
        config.expected.payments.len(),
        "Payment count {} != expected {}",
        payments.len(),
        config.expected.payments.len()
    );

    // 7. Verify each expected payment exists with matching attributes
    for expected in &config.expected.payments {
        // Match by payment ID
        let found = payments.iter().find(|p| p.id == expected.id);

        let payment =
            found.unwrap_or_else(|| panic!("Expected payment not found: id={}", expected.id));

        // Verify core fields (exact match since captured after finalization)
        assert_eq!(
            payment.method.to_string(),
            expected.method,
            "Method mismatch for {}: {} vs expected {}",
            expected.id,
            payment.method,
            expected.method
        );
        assert_eq!(
            payment.payment_type.to_string(),
            expected.payment_type,
            "PaymentType mismatch for {}: {} vs expected {}",
            expected.id,
            payment.payment_type,
            expected.payment_type
        );
        assert_eq!(
            payment.amount, expected.amount,
            "Amount mismatch for {}: {} vs expected {}",
            expected.id, payment.amount, expected.amount
        );
        assert_eq!(
            payment.status.to_string(),
            expected.status,
            "Status mismatch for {}: {} vs expected {}",
            expected.id,
            payment.status,
            expected.status
        );
        assert_eq!(
            payment.timestamp, expected.timestamp,
            "Timestamp mismatch for {}: {} vs expected {}",
            expected.id, payment.timestamp, expected.timestamp
        );
        assert_eq!(
            payment.fees, expected.fees,
            "Fees mismatch for {}: {} vs expected {}",
            expected.id, payment.fees, expected.fees
        );

        // Verify variant-specific details
        match &expected.details {
            Some(ExpectedPaymentDetails::SparkHtlc {
                payment_hash,
                preimage,
            }) => {
                if let Some(PaymentDetails::Spark {
                    htlc_details: Some(htlc),
                    ..
                }) = &payment.details
                {
                    assert_eq!(
                        &htlc.payment_hash, payment_hash,
                        "HTLC payment_hash mismatch for {}",
                        expected.id
                    );
                    assert_eq!(
                        &htlc.preimage, preimage,
                        "HTLC preimage mismatch for {}",
                        expected.id
                    );
                } else {
                    panic!("Expected Spark HTLC details for {}", expected.id);
                }
            }
            Some(ExpectedPaymentDetails::Lightning {
                payment_hash,
                preimage,
            }) => {
                if let Some(PaymentDetails::Lightning { htlc_details, .. }) = &payment.details {
                    assert_eq!(
                        &htlc_details.payment_hash, payment_hash,
                        "Lightning payment_hash mismatch for {}",
                        expected.id
                    );
                    assert_eq!(
                        &htlc_details.preimage, preimage,
                        "Lightning preimage mismatch for {}",
                        expected.id
                    );
                } else {
                    panic!("Expected Lightning details for {}", expected.id);
                }
            }
            Some(ExpectedPaymentDetails::OnChain { tx_id }) => match &payment.details {
                Some(PaymentDetails::Deposit { tx_id: t })
                | Some(PaymentDetails::Withdraw { tx_id: t }) => {
                    assert_eq!(t, tx_id, "OnChain tx_id mismatch for {}", expected.id);
                }
                _ => {
                    panic!("Expected OnChain details for {}", expected.id);
                }
            },
            Some(ExpectedPaymentDetails::Token { token_identifier }) => {
                if let Some(PaymentDetails::Token { metadata, .. }) = &payment.details {
                    assert_eq!(
                        &metadata.identifier, token_identifier,
                        "Token identifier mismatch for {}",
                        expected.id
                    );
                } else {
                    panic!("Expected Token details for {}", expected.id);
                }
            }
            None => {
                // No variant-specific assertions needed
            }
        }

        info!(
            "Verified {} {} payment: {} units, fees={}, timestamp={}",
            expected.payment_type,
            expected.method,
            expected.amount,
            expected.fees,
            expected.timestamp
        );
    }

    info!(
        "Recovery test passed: {} payments verified",
        config.expected.payments.len()
    );
    info!("=== Test test_wallet_recovery_from_mnemonic PASSED ===");
    Ok(())
}
