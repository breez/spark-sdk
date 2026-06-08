//! Integration tests for the Turnkey-backed signers.
//!
//! These require a Turnkey wallet that is already provisioned (the SDK does not
//! provision it) plus credentials in the environment:
//! `TURNKEY_ORG_ID`, `TURNKEY_API_PUBLIC_KEY`, `TURNKEY_API_PRIVATE_KEY`,
//! `TURNKEY_WALLET_ID` (`TURNKEY_BASE_URL` defaults to https://api.turnkey.com).
//! When any is unset, each test logs and returns early, so the suite is a no-op
//! without credentials. They also use the regtest faucet (`FAUCET_*` env), like
//! the rest of breez-itest.
//!
//! Coverage: connect + identity-key derivation, deposit funding (static-deposit
//! signing), outbound Spark transfer (prepare_transfer + sign_frost), and
//! inbound Spark claim (prepare_claim). Lightning receive and static-deposit
//! refund are not yet exercised here.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use tracing::info;

fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    seed
}

/// Builds the Turnkey-backed SDK, or `None` when credentials are absent.
async fn turnkey_sdk() -> Result<Option<SdkInstance>> {
    let temp = tempfile::TempDir::new()?;
    let dir = temp.path().to_string_lossy().to_string();
    build_sdk_with_turnkey(dir, Some(temp)).await
}

/// Builds a seed-based SDK to act as a counterparty in transfer tests.
async fn seed_sdk() -> Result<SdkInstance> {
    let temp = tempfile::TempDir::new()?;
    let dir = temp.path().to_string_lossy().to_string();
    build_sdk_with_dir(dir, random_seed(), Some(temp)).await
}

/// Connect with the Turnkey signers and read wallet info + a Spark address.
/// Exercises identity-key derivation and a clean signer-backed connect.
#[test_log::test(tokio::test)]
async fn test_turnkey_get_info_and_address() -> Result<()> {
    let Some(tk) = turnkey_sdk().await? else {
        return Ok(());
    };

    let info = tk
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;
    info!("Turnkey wallet balance: {} sats", info.balance_sats);

    let address = tk
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    assert!(!address.is_empty(), "expected a Spark address");
    info!("Turnkey Spark address: {address}");
    Ok(())
}

/// Fund the Turnkey wallet from the faucet (deposit + static-deposit signing)
/// and send a Spark transfer to a seed-based receiver (prepare_transfer +
/// sign_frost on the Turnkey Spark signer).
#[test_log::test(tokio::test)]
async fn test_turnkey_fund_and_send_spark() -> Result<()> {
    let Some(mut tk) = turnkey_sdk().await? else {
        return Ok(());
    };
    let mut bob = seed_sdk().await?;

    ensure_funded(&mut tk, 1000).await?;
    info!("Turnkey wallet funded");

    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = tk
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_address,
            amount: Some(100),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    let send = tk
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    assert_eq!(send.payment.payment_type, PaymentType::Send);

    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, 100);
    info!("Receiver got {} sats from Turnkey wallet", received.amount);
    Ok(())
}

/// Receive a Spark transfer into the Turnkey wallet from a seed-based sender
/// (exercises prepare_claim on the Turnkey Spark signer).
#[test_log::test(tokio::test)]
async fn test_turnkey_receive_spark() -> Result<()> {
    let Some(mut tk) = turnkey_sdk().await? else {
        return Ok(());
    };
    let mut alice = seed_sdk().await?;
    ensure_funded(&mut alice, 1000).await?;

    let tk_address = tk
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: tk_address,
            amount: Some(100),
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

    let received =
        wait_for_payment_succeeded_event(&mut tk.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, 100);
    info!("Turnkey wallet received {} sats", received.amount);
    Ok(())
}
