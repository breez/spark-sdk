use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tracing::info;

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fetch_lightning_send_status_from_a_restored_wallet(
    #[future] env: Result<Environment>,
) -> Result<()> {
    let env = env.await?;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut alice = env.create_wallet_from_seed(seed, |_| {}).await?;
    let bob = env.create_wallet().await?;
    ensure_funded(&mut alice, 100_000).await?;

    let invoice = create_invoice(&bob.sdk, "Lightning send status").await?;

    let unpaid = fetch_status(&alice.sdk, &invoice).await?;
    assert_eq!(unpaid.status, None);
    assert_eq!(unpaid.preimage, None);

    pay_invoice(&alice.sdk, &invoice, false).await?;
    let sent = wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
    let Some(PaymentDetails::Lightning { htlc_details, .. }) = sent.details else {
        panic!("expected Lightning payment details");
    };
    info!("Alice paid the invoice, payment {}", sent.id);

    let restored = env.create_wallet_from_seed(seed, |_| {}).await?;
    let status = fetch_status(&restored.sdk, &invoice).await?;
    assert_eq!(status.status, Some(PaymentStatus::Completed));
    assert!(status.preimage.is_some());
    assert_eq!(status.preimage, htlc_details.preimage);

    Ok(())
}

/// A Bolt11 paid by a direct Spark transfer to its embedded Spark address leaves no Lightning
/// payment on the operators, so the status stays unset.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_fetch_lightning_send_status_does_not_see_a_spark_route_payment(
    #[future] env: Result<Environment>,
) -> Result<()> {
    let env = env.await?;
    let mut alice = env.create_wallet().await?;
    let mut bob = env.create_wallet().await?;
    ensure_funded(&mut alice, 100_000).await?;

    let invoice = create_invoice(&bob.sdk, "Spark route send status").await?;
    pay_invoice(&alice.sdk, &invoice, true).await?;
    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.method, PaymentMethod::Spark);

    let status = fetch_status(&alice.sdk, &invoice).await?;
    assert_eq!(status.status, None);
    assert_eq!(status.preimage, None);

    Ok(())
}

async fn create_invoice(sdk: &BreezSdk, description: &str) -> Result<String> {
    Ok(sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: description.to_string(),
                amount_sats: Some(5_000),
                expiry_secs: None,
                payment_hash: None,
                receiver_identity_public_key: None,
                preimage: None,
            },
        })
        .await?
        .payment_request)
}

async fn pay_invoice(sdk: &BreezSdk, invoice: &str, prefer_spark: bool) -> Result<()> {
    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: invoice.to_string(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    sdk.send_payment(SendPaymentRequest {
        prepare_response,
        options: Some(SendPaymentOptions::Bolt11Invoice {
            prefer_spark,
            completion_timeout_secs: Some(10),
        }),
        idempotency_key: None,
    })
    .await?;
    Ok(())
}

async fn fetch_status(sdk: &BreezSdk, invoice: &str) -> Result<FetchLightningSendStatusResponse> {
    Ok(sdk
        .fetch_lightning_send_status(FetchLightningSendStatusRequest {
            invoice: invoice.to_string(),
        })
        .await?)
}
