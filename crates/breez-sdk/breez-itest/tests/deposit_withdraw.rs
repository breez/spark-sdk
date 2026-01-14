use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
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
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?
        .payment_request;
    info!("Bob deposit address: {}", bob_address);

    // Alice prepares and sends 15_000 sats on-chain to Bob
    let amount = 15_000u128;
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_address.clone(),
            amount: Some(amount),
            token_identifier: None,
            conversion_options: None,
        })
        .await?;

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::BitcoinAddress {
                confirmation_speed: OnchainConfirmationSpeed::Medium,
            }),
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
            payment_method: ReceivePaymentMethod::BitcoinAddress,
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
    assert!(matches!(
        claim_resp.payment.payment_type,
        PaymentType::Receive
    ));
    assert!(matches!(claim_resp.payment.method, PaymentMethod::Deposit));

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

/// Verify deposit no fee blocks auto-claim then refund
#[rstest]
#[ignore]
#[test_log::test(tokio::test)]
async fn test_deposit_fee_refund(#[future] bob_no_fee_sdk: Result<SdkInstance>) -> Result<()> {
    let mut bob = bob_no_fee_sdk.await?;

    // Acquire a static deposit address
    let addr = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
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

    // Refund to the same static address (acceptable for test)
    let refund_dest = addr.clone();
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
