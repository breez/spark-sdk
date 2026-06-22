//! Live Turnkey test for the gated bolt11 send flow.
//!
//! Prepares a gated send, manually submits its `prepare_transfer` activity on
//! the same Turnkey signer the SDK uses (creating the activity), then resumes
//! the send. With the test org auto-completing activities, the send's
//! `prepare_transfer` re-submits a byte-identical body (same params, same
//! timestamp from the shared activity store) and Turnkey folds it into the same
//! activity rather than creating a new one.
//!
//! The flow completing is the assertion. Each submission logs its activity id
//! ("Turnkey activity submitted: type=ACTIVITY_TYPE_SPARK_PREPARE_TRANSFER
//! key=... id=..."), so a run's logs show the manual call and the send
//! resolving to the same id. Requires the `TURNKEY_*` credentials (the
//! `turnkey` feature opts in) and a regtest backend.

#![cfg(feature = "turnkey")]

use anyhow::Result;
use breez_sdk_itest::turnkey::alice_turnkey_lightning_sdk;
use breez_sdk_itest::*;
use breez_sdk_spark::signer::ExternalSparkSigner;
use breez_sdk_spark::*;
use rstest::*;
use std::sync::Arc;

#[rstest]
#[test_log::test(tokio::test)]
async fn test_send_reuses_manual_prepare_transfer_activity(
    #[future] alice_turnkey_lightning_sdk: Result<(SdkInstance, Arc<dyn ExternalSparkSigner>)>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    let (mut alice, spark_signer) = alice_turnkey_lightning_sdk.await?;
    let bob = bob_sdk.await?;

    ensure_funded(&mut alice, 100_000).await?;

    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "turnkey prepare_transfer reuse".to_string(),
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
            fee_policy: Some(FeePolicy::FeesExcluded),
            include_transfer_context: Some(true),
        })
        .await?;
    let context = prepare
        .transfer_context
        .clone()
        .expect("a fees-excluded lightning send must produce a transfer context");

    // Submit the prepare_transfer activity manually, on the same signer the SDK
    // uses (so they share the activity-timestamp store). This logs its id.
    let authorization = alice
        .sdk
        .build_transfer_authorization_request(context.clone())
        .await?;
    spark_signer.prepare_transfer(authorization).await?;

    // Resume the send. Its prepare_transfer re-submits the identical body and
    // must resolve to the activity just created (logged with the same id). The
    // send completing is the assertion that the reused activity was usable.
    let send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare.clone(),
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(30),
            }),
            idempotency_key: None,
            transfer_context: Some(context),
        })
        .await?;
    assert!(matches!(
        send.payment.status,
        PaymentStatus::Completed | PaymentStatus::Pending
    ));

    Ok(())
}
