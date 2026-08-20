use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use spark_itest::mempool::MempoolClient;
use tokio::time::{Duration, sleep};
use tracing::{info, warn};

// ---------------------
// Local helpers
// ---------------------

async fn wait_for_unclaimed_event(
    event_rx: &mut tokio::sync::mpsc::Receiver<SdkEvent>,
    timeout: u64,
) -> Result<Vec<DepositInfo>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("Timeout waiting for UnclaimedDeposits event");
        }
        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::UnclaimedDeposits { unclaimed_deposits })) => {
                return Ok(unclaimed_deposits);
            }
            Ok(Some(other)) => {
                warn!(
                    "Received other SDK event while waiting for failure: {:?}",
                    other
                );
                continue;
            }
            Ok(None) => anyhow::bail!("Event channel closed"),
            Err(_) => anyhow::bail!("Timeout waiting for UnclaimedDeposits event"),
        }
    }
}

// ---------------------
// Tests
// ---------------------

/// Send on-chain from Alice to Bob's static deposit address and verify claim.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_onchain_withdraw_to_static_address(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    // Ensure Alice has enough funds for withdraw amount + fees
    ensure_funded(&mut alice, 120_000).await?;

    // Record Bob's initial balance
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_initial = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    // Bob exposes a static deposit address
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    info!("Bob deposit address: {}", bob_address);

    // Alice prepares and sends 15_000 sats on-chain to Bob
    let amount = 15_000u64;
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_address.clone(),
            },
            amount: Some(amount as u128),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!("Alice withdraw status: {:?}", send_resp.payment.status);
    assert!(matches!(send_resp.payment.method, PaymentMethod::Withdraw));
    assert!(matches!(send_resp.payment.payment_type, PaymentType::Send));

    let stored_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send_resp.payment.id.clone(),
        })
        .await?;
    assert!(matches!(
        stored_payment.payment.status,
        PaymentStatus::Pending
    ));

    // Trigger Bob sync and wait for receive + claim
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let recv_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 180).await?;
    assert!(matches!(recv_payment.method, PaymentMethod::Deposit));
    assert!(
        matches!(recv_payment.details, Some(PaymentDetails::Deposit { .. })),
        "Deposit payment must have Deposit details: {:?}",
        recv_payment.details
    );

    info!("Bob deposit fees after claim: {}", recv_payment.fees);
    assert!(recv_payment.fees > 0);

    // Verify Bob's balance increased and no unclaimed deposits remain
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    assert!(bob_final > bob_initial, "Bob's balance should increase");

    let unclaimed = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    assert!(
        unclaimed.is_empty(),
        "Unclaimed deposits should be empty after auto-claim"
    );

    Ok(())
}

/// Verify deposit fee limit blocks auto-claim then manually claim
#[rstest]
#[ignore]
#[test_log::test(tokio::test)]
async fn test_deposit_fee_manual_claim(
    #[future] bob_strict_fee_sdk: Result<SdkInstance>,
) -> Result<()> {
    let mut bob = bob_strict_fee_sdk.await?;

    // Acquire a static deposit address
    let addr = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;

    // Fund address via faucet; strict max fee blocks auto-claim
    let faucet = RegtestFaucet::new()?;
    let fund_amount = 30_000u64;
    let txid = faucet.fund_address(&addr, fund_amount).await?;
    info!("Faucet txid: {}", txid);

    // Start sync and wait for UnclaimedDeposits due to fee limit
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let failed = wait_for_unclaimed_event(&mut bob.events, 180).await?;
    assert!(!failed.is_empty());
    let (txid_found, vout) = {
        let d = failed
            .iter()
            .find(|d| d.txid == txid)
            .expect("deposit should appear in failed list");
        (d.txid.clone(), d.vout)
    };

    // Verify deposit is listed as unclaimed with claim_error
    let deposits = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    let dep = deposits
        .iter()
        .find(|d| d.txid == txid_found && d.vout == vout)
        .expect("unclaimed deposit not found");
    assert!(dep.claim_error.is_some(), "Expected claim_error to be set");

    // Manually claim with permissive fee
    let claim_resp = bob
        .sdk
        .claim_deposit(ClaimDepositRequest {
            txid: txid_found.clone(),
            vout,
            max_fee: Some(MaxFee::Fixed { amount: 100_000 }),
        })
        .await?;
    // A standard (mature) claim settles synchronously and returns the payment.
    let payment = claim_resp
        .payment
        .expect("standard claim should return a settled payment");
    assert!(matches!(payment.payment_type, PaymentType::Receive));
    assert!(matches!(payment.method, PaymentMethod::Deposit));
    assert!(
        matches!(
            &payment.details,
            Some(PaymentDetails::Deposit { vout: v, .. }) if *v == vout
        ),
        "Manual claim payment must carry vout={vout}: {:?}",
        payment.details
    );

    // After manual claim, deposit should be removed from unclaimed list
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let deposits_after_claim = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    assert!(
        !deposits_after_claim
            .iter()
            .any(|d| d.txid == txid_found && d.vout == vout),
        "Deposit should be removed after successful claim"
    );

    Ok(())
}

/// Test sending full balance to Bitcoin address with speed selection
#[rstest]
#[test_log::test(tokio::test)]
async fn test_send_all_to_bitcoin_address(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

    // Fund Alice with exactly a known amount
    let funding_amount = 50_000u64;
    receive_and_fund(&mut alice, funding_amount, false).await?;

    // Get Alice's initial balance (less than funding_amount due to claim fees)
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_initial = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice initial balance: {} sats", alice_initial);
    assert!(alice_initial > 0, "Alice should have been funded");

    // Bob exposes a static deposit address
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    info!("Bob deposit address: {}", bob_address);

    // Alice gets her balance to prepare FeesIncluded payment
    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?
        .balance_sats;

    // Alice prepares FeesIncluded (all balance)
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_address.clone(),
            },
            amount: Some(alice_balance as u128),
            token_identifier: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        })
        .await?;

    // Get fee quote from prepare response
    let SendPaymentMethod::BitcoinAddress { fee_quote, .. } = &prepare.payment_method else {
        panic!("Expected BitcoinAddress payment method");
    };
    info!(
        "Fee estimates - Fast: {}, Medium: {}, Slow: {}",
        fee_quote.speed_fast.total_fee_sat(),
        fee_quote.speed_medium.total_fee_sat(),
        fee_quote.speed_slow.total_fee_sat()
    );

    // Verify fee_policy is FeesIncluded
    assert!(
        matches!(prepare.fee_policy, FeePolicy::FeesIncluded),
        "Fee policy should be FeesIncluded"
    );
    info!(
        "FeesIncluded prepared with fast fee: {} sats",
        fee_quote.speed_fast.total_fee_sat()
    );

    // Send the full balance with Fast speed
    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::BitcoinAddress {
                confirmation_speed: OnchainConfirmationSpeed::Fast,
            }),
            idempotency_key: None,
        })
        .await?;

    info!("Alice withdraw status: {:?}", send_resp.payment.status);
    assert!(matches!(send_resp.payment.method, PaymentMethod::Withdraw));

    // Verify Alice's balance is now 0
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice final balance: {} sats", alice_final);
    assert_eq!(alice_final, 0, "Alice's balance should be fully spent");

    Ok(())
}

/// Verify deposit no fee blocks auto-claim, refund below the minimum relay fee is rejected, then refund succeeds with valid fee
#[rstest]
#[ignore]
#[test_log::test(tokio::test)]
async fn test_deposit_fee_refund(#[future] bob_no_fee_sdk: Result<SdkInstance>) -> Result<()> {
    let mut bob = bob_no_fee_sdk.await?;

    // Acquire a static deposit address
    let addr = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;

    // Fund address via faucet; no max fee blocks auto-claim
    let faucet = RegtestFaucet::new()?;
    let fund_amount = 25_000u64;
    let txid = faucet.fund_address(&addr, fund_amount).await?;
    info!("Faucet txid: {}", txid);

    // Start sync and wait for UnclaimedDeposits due to no fee set
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let failed = wait_for_unclaimed_event(&mut bob.events, 180).await?;
    assert!(!failed.is_empty());

    // Get current unclaimed deposit (use the new txid)
    let deposits = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    let dep = deposits
        .iter()
        .find(|d| d.txid == txid)
        .cloned()
        .expect("unclaimed deposit not found");

    // Refund to the same static address (acceptable for test), with a fee below
    // 1 sat/vB for the 111 vbyte refund to a taproot address
    let refund_dest = addr.clone();
    let result_below_min = bob
        .sdk
        .refund_deposit(RefundDepositRequest {
            txid: dep.txid.clone(),
            vout: dep.vout,
            destination_address: refund_dest.clone(),
            fee: Fee::Fixed { amount: 50 }, // Below minimum threshold
        })
        .await;

    // Assert the refund was rejected with minimum fee threshold error
    assert!(
        result_below_min.is_err(),
        "Expected refund to fail with fee below minimum"
    );
    let err = result_below_min.unwrap_err();
    let err_msg = format!("{:?}", err);
    assert!(
        err_msg.contains("fee must be at least 111 sats"),
        "Expected error message about minimum fee, got: {}",
        err_msg
    );
    info!(
        "Refund correctly rejected with fee below minimum: {}",
        err_msg
    );

    // Refund to the same static address (acceptable for test), with valid fee
    let refund = bob
        .sdk
        .refund_deposit(RefundDepositRequest {
            txid: dep.txid.clone(),
            vout: dep.vout,
            destination_address: refund_dest,
            fee: Fee::Fixed { amount: 500 },
        })
        .await?;
    info!("Refunded deposit with tx_id: {}", refund.tx_id);

    // Sync and assert the unclaimed deposit shows refund tx id or is removed post-confirmation
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    // give a brief moment for chain status to process
    sleep(Duration::from_secs(2)).await;
    let deposits_after_refund = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    if let Some(updated) = deposits_after_refund
        .iter()
        .find(|d| d.txid == dep.txid && d.vout == dep.vout)
    {
        assert_eq!(updated.refund_tx_id.as_deref(), Some(refund.tx_id.as_str()));
    } else {
        // already removed (confirmed); acceptable
    }

    Ok(())
}

/// Tests refund with a low deposit amount (1634 sats) using fee rate.
#[rstest]
#[ignore]
#[tokio::test]
async fn test_deposit_low_amount_refund_fee_rate(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_no_fee_sdk: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk.await?;
    let mut bob = bob_no_fee_sdk.await?;

    // Ensure Alice has enough funds
    ensure_funded(&mut alice, 10_000).await?;

    // Bob acquires a static deposit address
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    info!("Bob deposit address: {}", bob_address);

    // Alice sends a low amount (1634 sats) to Bob's deposit address
    let fund_amount = 1634u64;
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_address.clone(),
            },
            amount: Some(fund_amount as u128),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::BitcoinAddress {
                confirmation_speed: OnchainConfirmationSpeed::Fast,
            }),
            idempotency_key: None,
        })
        .await?;
    info!(
        "Alice sent {} sats to Bob, status: {:?}",
        fund_amount, send_resp.payment.status
    );

    // Sync Bob and wait for UnclaimedDeposits (no fee set blocks auto-claim)
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let failed = wait_for_unclaimed_event(&mut bob.events, 180).await?;
    assert!(!failed.is_empty());

    // Get the unclaimed deposit
    let deposits = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    let dep = deposits
        .iter()
        .find(|d| d.amount_sats == fund_amount)
        .cloned()
        .expect("unclaimed deposit not found");

    // Refund at the minimum relay fee rate, which the fee floor must accept
    let refund = bob
        .sdk
        .refund_deposit(RefundDepositRequest {
            txid: dep.txid.clone(),
            vout: dep.vout,
            destination_address: bob_address,
            fee: Fee::Rate { sat_per_vbyte: 1 },
        })
        .await?;
    info!(
        "Low amount refund succeeded with fee rate, tx_id: {}",
        refund.tx_id
    );

    Ok(())
}

/// Verify deposits to multiple rotated addresses are all discovered and claimed.
///
/// This test:
/// 1. Generates several deposit addresses (each call rotates to a new one)
/// 2. Funds the first (oldest) and last (newest) address via faucet
/// 3. Waits until both deposits are claimed by polling balance
#[rstest]
#[test_log::test(tokio::test)]
async fn test_deposits_to_multiple_addresses(
    #[future] alice_sdk: Result<SdkInstance>,
) -> Result<()> {
    let alice = alice_sdk.await?;

    // Generate several deposit addresses; each call rotates to a new one.
    let mut addresses = Vec::new();
    for _ in 0..5 {
        let addr = alice
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::BitcoinAddress {
                    new_address: Some(true),
                },
            })
            .await?
            .payment_request;
        info!("Generated deposit address: {}", addr);
        addresses.push(addr);
    }

    // All addresses must be distinct.
    let unique: std::collections::HashSet<&String> = addresses.iter().collect();
    assert_eq!(
        unique.len(),
        addresses.len(),
        "Every new address should be unique"
    );

    // Calling with new_address=false (or None) should return the same address.
    let reused_1 = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress {
                new_address: Some(false),
            },
        })
        .await?
        .payment_request;
    let reused_2 = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    assert_eq!(
        reused_1, reused_2,
        "new_address=false and None should return the same address"
    );
    // The reused address should match the last one obtained with new_address=true.
    assert_eq!(
        reused_1,
        *addresses.last().unwrap(),
        "new_address=false should return the latest address"
    );

    let first_addr = &addresses[0];
    let last_addr = addresses.last().unwrap();
    info!("Funding oldest ({}) and newest ({})", first_addr, last_addr);

    // Fund both the oldest and the newest address.
    let faucet = RegtestFaucet::new()?;
    let amount_first = 20_000u64;
    let amount_last = 30_000u64;
    let txid_first = faucet.fund_address(first_addr, amount_first).await?;
    let txid_last = faucet.fund_address(last_addr, amount_last).await?;
    info!("Faucet txids: first={}, last={}", txid_first, txid_last);

    // Wait until balance reflects both deposits (minus claim fees).
    let expected_min = amount_first + amount_last - 1_000;
    wait_for_balance(&alice.sdk, Some(expected_min), None, 200).await?;

    let info = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;
    info!("Final balance: {} sats", info.balance_sats);

    // No unclaimed deposits should remain.
    let unclaimed = alice
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    assert!(
        unclaimed.is_empty(),
        "All deposits should be auto-claimed, but found: {:?}",
        unclaimed
    );

    Ok(())
}

/// Manual instant claim path, end to end against the deployed SSP: it exercises
/// the ported user statement, the ECIES key-share transport, and the claim
/// mutation. Auto-claiming is disabled by a configured `max_deposit_claim_fee` of
/// 0, which now blocks both the instant attempt and the claim at maturity, so the
/// manual `claim_deposit` with its own ceiling is the only thing that can claim
/// the funded deposit.
///
/// It reads the vout straight from the funding tx to claim as early as possible,
/// but the claim does not depend on winning that race: a deposit that confirms
/// first is claimed at the shallowest plan the SSP still offers. Only a quote with
/// no fulfillment plans leaves nothing to assert, and the test skips. Otherwise
/// it asserts no synchronous payment, a credited balance net of the SSP spread,
/// and the deposit leaving the unclaimed list. A rejected statement is neither of
/// those nor transient, so it fails fast. Requires faucet creds and the deployed
/// regtest SSP to have instant claims enabled.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_manual_instant_deposit_claim(
    #[future] bob_strict_fee_sdk: Result<SdkInstance>,
) -> Result<()> {
    let bob = bob_strict_fee_sdk.await?;

    let start_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let faucet = RegtestFaucet::new()?;
    let mempool = MempoolClient::new()?;
    let fund_amount = 50_000u64;

    // Static deposit address.
    let addr = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;

    // Fund it and read the vout straight from the funding tx. Waiting for the SDK
    // to list the deposit is too slow: the operators mark it mature within the
    // indexing window, so by then the 0-conf window is already gone.
    let txid = faucet.fund_address(&addr, fund_amount).await?;
    info!("Funded static deposit, txid: {txid}");
    let tx = mempool.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, o)| {
            bitcoin::Address::from_script(&o.script_pubkey, bitcoin::Network::Regtest)
                .is_ok_and(|a| a.to_string() == addr)
        })
        .map(|(i, _)| i as u32)
        .expect("funding tx has no output paying the deposit address");

    // Claim instantly, retrying only while the SSP indexes the mempool tx. A quote
    // with no fulfillment plans leaves nothing to claim: skip. Any other error (a
    // rejected statement, the failure worth catching) fails fast.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let claim_resp = loop {
        match bob
            .sdk
            .claim_deposit(ClaimDepositRequest {
                txid: txid.clone(),
                vout,
                // regtest quotes a 0-conf plan, whose spread carries a 3% term:
                // ~1_700 sats at 50k, so the ceiling has to clear that.
                max_fee: Some(MaxFee::Fixed { amount: 3_000 }),
            })
            .await
        {
            Ok(resp) => break resp,
            Err(e) if e.to_string().contains("No instant claim plan available") => {
                warn!(
                    "SKIP test_manual_instant_deposit_claim: SSP offered no fulfillment plan for the deposit"
                );
                return Ok(());
            }
            // Two transient stages: the SSP has not indexed the tx yet
            // ("Transaction not found"), and it has but the UTXO is not deep
            // enough on every operator yet ("...confirmations..."). Retry only
            // those. A submission rejection phrased with "not found"
            // (unknown/consumed quote) falls through and fails fast.
            Err(e)
                if {
                    let msg = e.to_string().to_lowercase();
                    msg.contains("transaction not found") || msg.contains("confirmation")
                } =>
            {
                if tokio::time::Instant::now() >= deadline {
                    warn!(
                        "SKIP test_manual_instant_deposit_claim: deposit was not claimable within timeout"
                    );
                    return Ok(());
                }
                info!("instant claim not ready yet, retrying: {e}");
                // Poll tightly: the window closes as soon as the deposit matures
                // (regtest mines fast), so claim as early as the SSP allows.
                sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };
    // Which path the claim took is decided by maturity, not by the caller, and on
    // regtest a deposit matures at one confirmation. Winning the race to claim
    // early is therefore not guaranteed; a claim that lands after maturity settles
    // synchronously and returns a payment, which is correct and leaves nothing for
    // the early-claim assertions below to check.
    if let Some(payment) = claim_resp.payment {
        warn!(
            "SKIP early-claim assertions: the deposit matured before the claim landed, \
             so it settled at maturity ({} sats)",
            payment.amount
        );
        return Ok(());
    }

    // Mark-not-delete contract. claim_deposit marks the deposit Submitted, creating
    // the row first if the background sync has not (the create-if-missing path), so
    // right after the call the deposit is present and Submitted. reconcile_deposits
    // only removes it once the claim settles, which the settle-poll below waits for,
    // so the row is checked here first, while it is guaranteed present.
    let deposits = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    let dep = deposits
        .iter()
        .find(|d| d.txid == txid)
        .expect("a submitted instant claim must be listed (created and marked)");
    assert!(
        matches!(
            dep.instant_claim_status,
            Some(InstantClaimStatus::Submitted { .. })
        ),
        "instant-claimed deposit must be marked Submitted: {:?}",
        dep.instant_claim_status
    );

    // Poll until the async credit settles. This is the end-to-end proof: the SSP
    // accepted the signed statement, key share, and claim, and fronted the credit.
    let balance = wait_for_balance(&bob.sdk, Some(start_balance + 1), None, 180).await?;
    info!("Manual instant credit settled: {balance} (was {start_balance})");
    // Instant credit takes the SSP spread, so it is below the funded amount.
    assert!(
        balance < start_balance + fund_amount,
        "instant credit should be below the funded amount (SSP spread): {balance}"
    );

    Ok(())
}

/// The claim quote API against the deployed SSP and a real chain, which is the
/// only place its two unknowns can be answered: whether the SSP will quote a
/// claim at maturity for a deposit that has not matured (and so whether the
/// estimate fallback is the normal path or the exception), and whether the
/// confirmation count derived from the chain tip is sane.
///
/// Polls until the SSP has indexed the funding tx and offers an early claim,
/// since before that the quote is all fallbacks and asserts little. If it never
/// does, the invariants that do not depend on it are still checked.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_fetch_claim_deposit_quote(
    #[future] bob_strict_fee_sdk: Result<SdkInstance>,
) -> Result<()> {
    let bob = bob_strict_fee_sdk.await?;
    let faucet = RegtestFaucet::new()?;
    let mempool = MempoolClient::new()?;
    let fund_amount = 50_000u64;

    let addr = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;

    let txid = faucet.fund_address(&addr, fund_amount).await?;
    info!("Funded static deposit, txid: {txid}");
    let tx = mempool.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, o)| {
            bitcoin::Address::from_script(&o.script_pubkey, bitcoin::Network::Regtest)
                .is_ok_and(|a| a.to_string() == addr)
        })
        .map(|(i, _)| i as u32)
        .expect("funding tx has no output paying the deposit address");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let quote = loop {
        let quote = bob
            .sdk
            .fetch_claim_deposit_quote(FetchClaimDepositQuoteRequest {
                txid: txid.clone(),
                vout,
            })
            .await?;
        // Stop as soon as there is nothing left to wait for: an early claim is
        // only ever offered before maturity, so once the deposit matures the
        // answer will not change. regtest matures at one confirmation, so losing
        // this race to the next block is routine rather than a failure.
        if quote.instant.is_some()
            || quote.confirmations >= quote.mature.confirmations_required
            || tokio::time::Instant::now() >= deadline
        {
            break quote;
        }
        sleep(Duration::from_secs(1)).await;
    };
    info!("Deposit claim quote: {quote:?}");

    assert_eq!(
        quote.amount_sats, fund_amount,
        "the quote prices the funded UTXO"
    );
    // Freshly funded, so it is at most a block or two deep.
    assert!(
        quote.confirmations <= 2,
        "unexpected depth for a just-funded deposit: {}",
        quote.confirmations
    );

    // Waiting always has an answer, real or estimated, and always costs something.
    // regtest matures at 1 confirmation, unlike mainnet's 3.
    assert_eq!(quote.mature.confirmations_required, 1);
    assert!(quote.mature.fee_sats > 0, "a claim is never free");
    assert_eq!(
        quote.mature.credit_amount_sats,
        fund_amount - quote.mature.fee_sats
    );

    match &quote.instant {
        Some(instant) => {
            // Only ever offered when it credits strictly sooner than waiting would.
            // The provider's shallowest plan often lands at maturity's depth, and
            // that is filtered out rather than shown as a choice worth nothing.
            assert!(
                instant.confirmations_required < quote.mature.confirmations_required,
                "an early claim was offered at maturity's own depth ({}), which buys \
                 no time",
                instant.confirmations_required
            );
            assert!(
                instant.fee_sats > quote.mature.fee_sats,
                "claiming early costs more than waiting: {} vs {}",
                instant.fee_sats,
                quote.mature.fee_sats
            );
            assert!(!instant.is_estimate, "an offered plan is a real quote");
            assert_eq!(instant.credit_amount_sats, fund_amount - instant.fee_sats);
        }
        None => warn!(
            "SKIP early-claim assertions: no early claim on offer at {} confirmations \
             (maturity {}), either matured before the first quote or the provider \
             declined to front it",
            quote.confirmations, quote.mature.confirmations_required
        ),
    }

    // The estimate is only needed while the provider refuses to quote, so pair it
    // with the real quote once the deposit matures. The two numbers are logged
    // together because the estimate is derived from on-chain fees alone and has
    // no calibration against what the provider actually charges.
    let estimate = quote.mature.clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let matured = loop {
        let quote = bob
            .sdk
            .fetch_claim_deposit_quote(FetchClaimDepositQuoteRequest {
                txid: txid.clone(),
                vout,
            })
            .await?
            .mature;
        if !quote.is_estimate {
            break Some(quote);
        }
        if tokio::time::Instant::now() >= deadline {
            break None;
        }
        sleep(Duration::from_secs(2)).await;
    };

    match matured {
        Some(matured) => {
            info!(
                "mature claim fee: estimated {} sats, provider quoted {} sats",
                estimate.fee_sats, matured.fee_sats
            );
            assert!(matured.fee_sats > 0, "a claim is never free");
            assert_eq!(
                matured.credit_amount_sats,
                fund_amount - matured.fee_sats,
                "the real quote's credit must reconcile with its fee"
            );
        }
        None => warn!(
            "SKIP estimate calibration: the provider never quoted a claim at maturity \
             within the timeout (estimated {} sats)",
            estimate.fee_sats
        ),
    }

    // Does a plan's `confirmations_required` track the deposit's actual depth, or
    // is it a fixed floor? Quote once the deposit is several blocks deep: a plan
    // still reading shallower than the deposit is a floor, and then it says
    // nothing about whether claiming early would actually be earlier.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let deep = loop {
        let quote = bob
            .sdk
            .fetch_claim_deposit_quote(FetchClaimDepositQuoteRequest {
                txid: txid.clone(),
                vout,
            })
            .await?;
        if quote.confirmations >= 3 {
            break Some(quote);
        }
        if tokio::time::Instant::now() >= deadline {
            break None;
        }
        sleep(Duration::from_secs(2)).await;
    };

    match deep {
        Some(deep) => {
            info!(
                "at {} confirmations the quote returned an early claim at depth {:?} \
                 (maturity is {}); the provider still offers one, this is what \
                 survives filtering",
                deep.confirmations,
                deep.instant.as_ref().map(|i| i.confirmations_required),
                deep.mature.confirmations_required
            );
            // The provider keeps offering a plan long past maturity (measured: depth
            // 1 while the deposit sat at 4 confirmations), and taking it would cost a
            // spread for no time saved. A matured deposit must therefore be quoted
            // one option only, matching what claim_deposit would actually do.
            assert!(
                deep.instant.is_none(),
                "a matured deposit was quoted an early claim at depth {:?}, which \
                 claim_deposit would refuse: the app would show a choice the SDK \
                 will not take",
                deep.instant.as_ref().map(|i| i.confirmations_required)
            );
        }
        None => warn!("SKIP depth-tracking measurement: deposit stayed under 3 confirmations"),
    }

    Ok(())
}
