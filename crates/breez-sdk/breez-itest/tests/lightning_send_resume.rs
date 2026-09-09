//! Finishing a Lightning send whose SSP request never landed.

use anyhow::{Result, bail};
use breez_sdk_itest::fixtures::ssp_fault::SspFaultProxy;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tracing::info;

const SEND_SATS: u64 = 1_000;

/// The SSP mutation that asks for the invoice to be paid. Losing just this one
/// is the whole point: the operators have already committed the leaves by the
/// time it is sent.
const LIGHTNING_SEND_OPERATION: &str = "RequestLightningSend";

fn regtest_ssp_base_url() -> String {
    default_config(Network::Regtest)
        .spark_config
        .expect("default_config populates spark_config")
        .ssp_config
        .base_url
}

/// Rebuilds the sender on the same storage and tree-store backend, so state
/// persists across the restart, pointed at the fault proxy rather than the real
/// SSP.
async fn build_sender(
    storage_dir: &str,
    seed: [u8; 32],
    backend: BackendChoice,
    ssp_base_url: &str,
) -> Result<SdkInstance> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.prefer_spark_over_lightning = false;
    config.sync_interval_secs = 1;
    config
        .spark_config
        .as_mut()
        .expect("default_config populates spark_config")
        .ssp_config
        .base_url = ssp_base_url.to_string();
    build_sdk_with_custom_config_and_backend(
        storage_dir.to_string(),
        seed,
        config,
        None,
        false,
        Some(backend),
    )
    .await
}

async fn balance_of(sdk: &BreezSdk) -> Result<u64> {
    sdk.sync_wallet(SyncWalletRequest {}).await?;
    Ok(sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats)
}

async fn completed_lightning_sends(sdk: &BreezSdk) -> Result<usize> {
    Ok(sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Completed]),
            type_filter: Some(vec![PaymentType::Send]),
            ..Default::default()
        })
        .await?
        .payments
        .len())
}

/// A send commits leaves to the operators under a preimage condition, and only
/// the SSP request that follows settles them. When that request never lands the
/// amount is stuck until the operators expire it 16 days later, so the SDK
/// records the send beforehand and finishes it on a later sync.
///
/// The orphan is produced by failing only `RequestLightningSend`: the send has
/// to reach the operators to commit anything, and refusing the whole SSP would
/// instead fail the fee estimate that `validate_payment` asks for first.
/// Rebuilding the sender between the two halves makes this a real restart,
/// which is the case the record exists for. A resume is the only thing that can
/// pay the invoice afterwards, so the receiver being paid at the end is what
/// proves it ran.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_send_orphaned_before_the_ssp_resumes_on_sync() -> Result<()> {
    let receiver_dir = tempfile::Builder::new()
        .prefix("breez-sdk-resume-receiver")
        .tempdir()?;
    let sender_dir = tempfile::Builder::new()
        .prefix("breez-sdk-resume-sender")
        .tempdir()?;
    let sender_path = sender_dir.path().to_string_lossy().to_string();

    let mut receiver_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut receiver_seed);
    let mut sender_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sender_seed);

    let receiver = build_sdk_with_dir(
        receiver_dir.path().to_string_lossy().to_string(),
        receiver_seed,
        Some(receiver_dir),
    )
    .await?;
    let backend = resolve_backend_choice().await?;
    let ssp = SspFaultProxy::start(&regtest_ssp_base_url(), LIGHTNING_SEND_OPERATION).await?;

    // Fund the sender and prepare while the SSP works end to end.
    let mut sender =
        build_sender(&sender_path, sender_seed, backend.clone(), &ssp.base_url()).await?;
    ensure_funded(&mut sender, 10_000).await?;

    let invoice = receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "resume an orphaned send".to_string(),
                amount_sats: Some(SEND_SATS),
                expiry_secs: None,
                payment_hash: None,
                receiver_identity_public_key: None,
            },
        })
        .await?
        .payment_request;

    let prepare = sender
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: invoice.clone(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    sender.sdk.disconnect().await?;

    // Lose the send request before the SSP sees it. The operators still commit
    // the leaves, so the invoice is unpaid and only a resume can settle it.
    ssp.start_failing();
    let sender = build_sender(&sender_path, sender_seed, backend.clone(), &ssp.base_url()).await?;
    let orphaned = sender
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(10),
            }),
            idempotency_key: None,
        })
        .await;
    assert!(
        orphaned.is_err(),
        "the send must fail while its SSP request is lost, got {orphaned:?}"
    );
    assert_eq!(
        completed_lightning_sends(&sender.sdk).await?,
        0,
        "nothing may be recorded as paid while the SSP was never asked"
    );
    let receiver_before = balance_of(&receiver.sdk).await?;
    sender.sdk.disconnect().await?;

    // Restart with the SSP working again. Only a resume can pay the invoice now.
    ssp.stop_failing();
    let sender = build_sender(&sender_path, sender_seed, backend, &ssp.base_url()).await?;
    info!("Waiting for the resumed send to reach the receiver");
    wait_for(
        || async {
            sender.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let balance = balance_of(&receiver.sdk).await?;
            if balance >= receiver_before + SEND_SATS {
                return Ok(balance);
            }
            bail!(
                "receiver still at {balance}, want {}",
                receiver_before + SEND_SATS
            );
        },
        120,
    )
    .await?;

    assert_eq!(
        completed_lightning_sends(&sender.sdk).await?,
        1,
        "the resumed send must be recorded as a completed payment"
    );

    sender.sdk.disconnect().await?;
    receiver.sdk.disconnect().await?;
    Ok(())
}

/// A send request the SSP accepted but whose response was lost is repeated: by
/// the client's own retry, and by a resume that cannot tell the two cases apart.
/// The transfer id travels with it so the SSP resolves the repeat to the request
/// it already has. Were it not to, the same invoice would be paid twice, from
/// two sets of leaves.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_repeated_send_requests_resolve_to_one() -> Result<()> {
    let receiver_dir = tempfile::Builder::new()
        .prefix("breez-sdk-repeat-receiver")
        .tempdir()?;
    let sender_dir = tempfile::Builder::new()
        .prefix("breez-sdk-repeat-sender")
        .tempdir()?;
    let sender_path = sender_dir.path().to_string_lossy().to_string();

    let mut receiver_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut receiver_seed);
    let mut sender_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sender_seed);

    let receiver = build_sdk_with_dir(
        receiver_dir.path().to_string_lossy().to_string(),
        receiver_seed,
        Some(receiver_dir),
    )
    .await?;
    let backend = resolve_backend_choice().await?;
    let ssp = SspFaultProxy::start(&regtest_ssp_base_url(), LIGHTNING_SEND_OPERATION).await?;

    let mut sender = build_sender(&sender_path, sender_seed, backend, &ssp.base_url()).await?;
    ensure_funded(&mut sender, 10_000).await?;

    let invoice = receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "repeat a send request".to_string(),
                amount_sats: Some(SEND_SATS),
                expiry_secs: None,
                payment_hash: None,
                receiver_identity_public_key: None,
            },
        })
        .await?
        .payment_request;

    let prepare = sender
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: invoice.clone(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let receiver_before = balance_of(&receiver.sdk).await?;

    // The SSP handles every send request; only the responses are lost, so the
    // client retries into an SSP that has already seen the request.
    ssp.start_losing_responses();
    let lost = sender
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(10),
            }),
            idempotency_key: None,
        })
        .await;
    assert!(
        lost.is_err(),
        "the send must fail while its responses are lost, got {lost:?}"
    );
    ssp.stop_failing();

    let request_ids = ssp.request_ids();
    info!(
        "SSP saw {} send request(s): {request_ids:?}",
        request_ids.len()
    );
    assert!(
        request_ids.len() > 1,
        "the client must have repeated the request for this to prove anything, saw {request_ids:?}"
    );
    assert!(
        request_ids.windows(2).all(|pair| pair[0] == pair[1]),
        "the SSP opened a separate send request per repeat: {request_ids:?}"
    );

    // One request means one payment, however many times it was asked for.
    wait_for(
        || async {
            sender.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let balance = balance_of(&receiver.sdk).await?;
            if balance >= receiver_before + SEND_SATS {
                return Ok(balance);
            }
            bail!("receiver still at {balance}");
        },
        120,
    )
    .await?;
    let settled = balance_of(&receiver.sdk).await?;
    assert_eq!(
        settled,
        receiver_before + SEND_SATS,
        "the invoice was paid more than once"
    );

    sender.sdk.disconnect().await?;
    receiver.sdk.disconnect().await?;
    Ok(())
}
