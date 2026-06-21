//! Signer-backend-parametrized integration tests.
//!
//! Each flow runs against every signer backend, so seed-based and Turnkey-backed
//! signers share a single test body (see [`SignerBackend`] /
//! `build_backend_sdk`). The Turnkey cases only exist when this crate is built
//! with the `turnkey` feature; they then require `TURNKEY_*` credentials and
//! fail without them. All cases use the regtest faucet, like the rest of
//! breez-itest, so they run in CI rather than locally.
//!
//! Coverage per backend: identity-key derivation (`info_and_address`), on-chain
//! deposit funding / static-deposit signing (`fund_onchain_deposit`), and an
//! outbound transfer + inbound claim (`send_receive_spark`).

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use rstest_reuse::{apply, template};
use tracing::info;

// The templates exist in two variants: with the Turnkey cases when this crate
// is built with the `turnkey` feature, and seed-only otherwise, so default
// builds do not even contain the Turnkey cases.

/// Single-party flows: run once per backend.
#[cfg(feature = "turnkey")]
#[template]
#[rstest]
#[case::seed(SignerBackend::Seed)]
#[case::turnkey(SignerBackend::Turnkey)]
fn each_backend(#[case] backend: SignerBackend) {}

/// Single-party flows: run once per backend.
#[cfg(not(feature = "turnkey"))]
#[template]
#[rstest]
#[case::seed(SignerBackend::Seed)]
fn each_backend(#[case] backend: SignerBackend) {}

/// Two-party transfers, pairing Turnkey with a seed-based counterparty in both
/// directions.
#[cfg(feature = "turnkey")]
#[template]
#[rstest]
#[case::seed_to_seed(SignerBackend::Seed, SignerBackend::Seed)]
#[case::turnkey_to_seed(SignerBackend::Turnkey, SignerBackend::Seed)]
#[case::seed_to_turnkey(SignerBackend::Seed, SignerBackend::Turnkey)]
fn transfer_backends(#[case] sender: SignerBackend, #[case] receiver: SignerBackend) {}

/// Two-party transfers.
#[cfg(not(feature = "turnkey"))]
#[template]
#[rstest]
#[case::seed_to_seed(SignerBackend::Seed, SignerBackend::Seed)]
fn transfer_backends(#[case] sender: SignerBackend, #[case] receiver: SignerBackend) {}

/// Connect, read wallet info, and derive a Spark address (identity-key path).
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn info_and_address(#[case] backend: SignerBackend) -> Result<()> {
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
            payment_request: PaymentRequest::Input { input: rx_address },
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

/// Receive a Lightning payment: `backend` creates a Bolt11 invoice (exercising
/// prepare_lightning_receive) and a seed-based sender pays it over Lightning.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn lightning_receive(#[case] backend: SignerBackend) -> Result<()> {
    let mut receiver = build_backend_sdk(backend).await?;
    let mut sender = build_backend_sdk(SignerBackend::Seed).await?;
    ensure_funded(&mut sender, 5000).await?;

    let invoice = receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "turnkey itest".to_string(),
                amount_sats: Some(100),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = sender
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input { input: invoice },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    sender
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(10),
            }),
            idempotency_key: None,
        })
        .await?;

    let received =
        wait_for_payment_succeeded_event(&mut receiver.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, 100);
    info!(
        "[{backend:?}] received {} sats over Lightning",
        received.amount
    );
    Ok(())
}

/// Refund an unclaimed on-chain deposit (exercises start_static_deposit_refund +
/// sign_static_deposit_refund). Auto-claim is blocked via `max_deposit_claim_fee
/// = None` so the deposit stays unclaimed and can be refunded on-chain.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn static_deposit_refund(#[case] backend: SignerBackend) -> Result<()> {
    let mut config = regtest_test_config();
    config.max_deposit_claim_fee = None;
    let mut sdk = build_backend_sdk_with_config(backend, config).await?;

    let address = sdk
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;

    let faucet = RegtestFaucet::new()?;
    let txid = faucet.fund_address(&address, 25_000).await?;
    info!("[{backend:?}] funded deposit {txid}, awaiting unclaimed event");

    sdk.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let unclaimed = wait_for_unclaimed_event(&mut sdk.events, 180).await?;
    assert!(!unclaimed.is_empty(), "expected an unclaimed deposit");

    let deposit = sdk
        .sdk
        .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
        .await?
        .deposits
        .into_iter()
        .find(|d| d.txid == txid)
        .ok_or_else(|| anyhow::anyhow!("unclaimed deposit not found"))?;

    // Refund to the same static address (acceptable for the test).
    let refund = sdk
        .sdk
        .refund_deposit(RefundDepositRequest {
            txid: deposit.txid,
            vout: deposit.vout,
            destination_address: address,
            fee: Fee::Rate { sat_per_vbyte: 2 },
        })
        .await?;
    assert!(!refund.tx_id.is_empty(), "expected a refund tx id");
    info!("[{backend:?}] refunded deposit via {}", refund.tx_id);
    Ok(())
}

/// Issue and mint a token (exercises prepare_token_transaction). The Turnkey
/// wallet persists across runs, so a pre-existing issuer token is tolerated:
/// only the mint, which signs, is required.
#[apply(each_backend)]
#[test_log::test(tokio::test)]
async fn token_mint(#[case] backend: SignerBackend) -> Result<()> {
    let sdk = build_backend_sdk(backend).await?;
    let issuer = sdk.sdk.get_token_issuer();

    let token_id = match issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "breez turnkey itest".to_string(),
            ticker: "BTKI".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply: 1_000_000_000,
        })
        .await
    {
        Ok(metadata) => metadata.identifier,
        Err(e) => {
            info!("create_issuer_token failed ({e}); assuming token already exists");
            sdk.sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(true),
                })
                .await?
                .token_balances
                .keys()
                .next()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no existing issuer token to mint"))?
        }
    };

    let before = sdk
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map_or(0, |b| b.balance);

    issuer
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1000 })
        .await?;

    let after = wait_for_token_balance_increase(&sdk.sdk, &token_id, before, 30).await?;
    assert!(after > before, "expected minted balance to increase");
    info!("[{backend:?}] minted token {token_id}: {before} -> {after}");
    Ok(())
}
