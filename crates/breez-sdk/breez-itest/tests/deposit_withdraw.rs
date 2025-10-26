use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tempdir::TempDir;
use tokio::time::{Duration, sleep};
use tracing::{info, warn};

// ---------------------
// Fixtures
// ---------------------

#[fixture]
async fn alice_sdk() -> Result<SdkInstance> {
    let dir = TempDir::new("breez-sdk-alice-onchain")?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    build_sdk_with_dir(path, seed, Some(dir)).await
}

#[fixture]
async fn bob_sdk() -> Result<SdkInstance> {
    let dir = TempDir::new("breez-sdk-bob-onchain")?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    build_sdk_with_dir(path, seed, Some(dir)).await
}

#[fixture]
async fn bob_strict_fee_sdk() -> Result<SdkInstance> {
    let dir = TempDir::new("breez-sdk-bob-fee")?;
    let path = dir.path().to_string_lossy().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut cfg = default_config(Network::Regtest);
    cfg.max_deposit_claim_fee = Some(Fee::Fixed { amount: 0 });
    build_sdk_with_custom_config(path, seed, cfg, Some(dir)).await
}

// ---------------------
// Local helpers
// ---------------------

async fn ensure_funded(sdk_instance: &mut SdkInstance, min_balance: u64) -> Result<()> {
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    if info.balance_sats < min_balance {
        let needed = min_balance - info.balance_sats;
        info!("Funding wallet via faucet: need {} sats", needed);
        receive_and_fund(sdk_instance, 50_000).await?;
    }
    Ok(())
}

async fn wait_for_claim_failed(
    event_rx: &mut tokio::sync::mpsc::Receiver<SdkEvent>,
    timeout: u64,
) -> Result<Vec<DepositInfo>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("Timeout waiting for ClaimDepositsFailed event");
        }
        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::ClaimDepositsFailed { unclaimed_deposits })) => {
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
            Err(_) => anyhow::bail!("Timeout waiting for ClaimDepositsFailed event"),
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
        })
        .await?;

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::BitcoinAddress {
                confirmation_speed: OnchainConfirmationSpeed::Medium,
            }),
        })
        .await?;

    info!("Alice withdraw status: {:?}", send_resp.payment.status);
    assert!(matches!(send_resp.payment.method, PaymentMethod::Withdraw));
    assert!(matches!(send_resp.payment.payment_type, PaymentType::Send));

    // Trigger Bob sync and wait for receive + claim
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let recv_payment = wait_for_payment_event(&mut bob.events, PaymentType::Receive, 180).await?;
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

/// Verify deposit fee limit blocks auto-claim, then manual claim succeeds; then refund path.
#[rstest]
#[ignore]
#[test_log::test(tokio::test)]
async fn test_deposit_fee_claim_and_refund(
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

    // Kick sync and wait for ClaimDepositsFailed due to fee limit
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let failed = wait_for_claim_failed(&mut bob.events, 180).await?;
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
            max_fee: Some(Fee::Fixed { amount: 100_000 }),
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

    // Fund again to test refund path (still strict fee blocks auto-claim)
    let txid2 = faucet.fund_address(&addr, 25_000).await?;
    info!("Faucet txid (for refund): {}", txid2);
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let _ = wait_for_claim_failed(&mut bob.events, 180).await?;

    // Get current unclaimed deposit (use the new txid)
    let deposits2 = bob
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits;
    let dep2 = deposits2
        .iter()
        .find(|d| d.txid == txid2)
        .cloned()
        .expect("second unclaimed deposit missing");

    // Refund to the same static address (acceptable for test)
    let refund_dest = addr.clone();
    let refund = bob
        .sdk
        .refund_deposit(RefundDepositRequest {
            txid: dep2.txid.clone(),
            vout: dep2.vout,
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
        .find(|d| d.txid == dep2.txid && d.vout == dep2.vout)
    {
        assert_eq!(updated.refund_tx_id.as_deref(), Some(refund.tx_id.as_str()));
    } else {
        // already removed (confirmed); acceptable
    }

    Ok(())
}
