//! Signer-backend-parametrized integration tests.
//!
//! Each flow runs against every signer backend, so seed-based and Turnkey-backed
//! signers share a single test body (see [`SignerBackend`] / `build_backend_sdk`).
//! Turnkey cases need a provisioned wallet plus `TURNKEY_*` credentials; without
//! them the case skips. All cases use the regtest faucet, like the rest of
//! breez-itest, so they run in CI rather than locally.
//!
//! Coverage per backend: identity-key derivation (`info_and_address`), on-chain
//! deposit funding / static-deposit signing (`fund_onchain_deposit`), and an
//! outbound transfer + inbound claim (`send_receive_spark`). The Turnkey wallet
//! is a single provisioned wallet, so transfers pair it with a seed-based
//! counterparty rather than a second Turnkey wallet.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use rstest_reuse::{apply, template};
use tracing::info;

/// Returns `true` (and logs) when a case involves the Turnkey backend but no
/// credentials are configured, so the case should skip without failing.
fn skip_without_turnkey(backends: &[SignerBackend]) -> bool {
    if backends.contains(&SignerBackend::Turnkey) && turnkey_config_from_env().is_none() {
        info!("Turnkey credentials unavailable; skipping case");
        return true;
    }
    false
}

/// Single-party flows: run once per backend.
#[template]
#[rstest]
#[case::seed(SignerBackend::Seed)]
#[case::turnkey(SignerBackend::Turnkey)]
fn each_backend(#[case] backend: SignerBackend) {}

/// Two-party transfers. The Turnkey wallet is a single provisioned wallet, so it
/// can be at most one side (no turnkey-to-turnkey case).
#[template]
#[rstest]
#[case::seed_to_seed(SignerBackend::Seed, SignerBackend::Seed)]
#[case::turnkey_to_seed(SignerBackend::Turnkey, SignerBackend::Seed)]
#[case::seed_to_turnkey(SignerBackend::Seed, SignerBackend::Turnkey)]
fn transfer_backends(#[case] sender: SignerBackend, #[case] receiver: SignerBackend) {}

/// Connect, read wallet info, and derive a Spark address (identity-key path).
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn info_and_address(#[case] backend: SignerBackend) -> Result<()> {
    if skip_without_turnkey(&[backend]) {
        return Ok(());
    }
    let sdk = build_backend_sdk(backend).await?;
    let info = sdk
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;
    info!("[{backend:?}] balance: {} sats", info.balance_sats);
    let address = sdk
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    assert!(!address.is_empty(), "expected a Spark address");
    Ok(())
}

/// Fund the wallet from the faucet via an on-chain deposit (deposit address +
/// static-deposit claim signing).
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn fund_onchain_deposit(#[case] backend: SignerBackend) -> Result<()> {
    if skip_without_turnkey(&[backend]) {
        return Ok(());
    }
    let mut sdk = build_backend_sdk(backend).await?;
    ensure_funded(&mut sdk, 1000).await?;
    let info = sdk
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    assert!(info.balance_sats >= 1000, "expected funded balance");
    info!("[{backend:?}] funded balance: {} sats", info.balance_sats);
    Ok(())
}

/// Send a Spark transfer from `sender` to `receiver` (prepare_transfer +
/// sign_frost on the sender, prepare_claim on the receiver).
#[apply(transfer_backends)]
#[test_log::test(tokio::test)]
async fn send_receive_spark(
    #[case] sender: SignerBackend,
    #[case] receiver: SignerBackend,
) -> Result<()> {
    if skip_without_turnkey(&[sender, receiver]) {
        return Ok(());
    }
    let mut tx = build_backend_sdk(sender).await?;
    let mut rx = build_backend_sdk(receiver).await?;

    ensure_funded(&mut tx, 1000).await?;

    let rx_address = rx
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = tx
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: rx_address,
            amount: Some(100),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    let send = tx
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    assert_eq!(send.payment.payment_type, PaymentType::Send);

    let received =
        wait_for_payment_succeeded_event(&mut rx.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, 100);
    info!(
        "[{sender:?} -> {receiver:?}] received {} sats",
        received.amount
    );
    Ok(())
}
