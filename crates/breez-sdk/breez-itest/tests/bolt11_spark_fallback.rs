//! A Bolt11 invoice that advertises a Spark destination, paid over Spark.
//!
//! The point of the fallback is that the receiver can still tell the payment
//! settled the invoice it issued, even though nothing reached the SSP.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Alice pays Bob's Bolt11 over Spark, and Bob sees his invoice paid.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_bolt11_settled_over_spark_is_attributed(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    const AMOUNT_SATS: u64 = 10;

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    ensure_funded(&mut alice, 100).await?;

    let invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "fallback attribution".to_string(),
                amount_sats: Some(AMOUNT_SATS),
                expiry_secs: Some(3600),
                payment_hash: None,
                receiver_identity_public_key: None,
            },
        })
        .await?
        .payment_request;
    info!("Bob's invoice: {invoice}");

    // Paying it must take the Spark route, which costs nothing.
    let prepare = alice
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
    let SendPaymentMethod::Bolt11Invoice {
        spark_transfer_fee_sats,
        ..
    } = &prepare.payment_method
    else {
        anyhow::bail!("expected a Bolt11 send method");
    };
    assert_eq!(
        *spark_transfer_fee_sats,
        Some(0),
        "the invoice should advertise a Spark destination"
    );

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Bob's payment must report as the Bolt11 being paid, not as a bare Spark
    // receive, and must carry no HTLC: none was ever created.
    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, u128::from(AMOUNT_SATS));

    // Read back rather than trusting the event: whichever ingestion path sees
    // the transfer first, storage is what a caller lists and waits on.
    let stored = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: received.id.clone(),
        })
        .await?
        .payment;
    assert_eq!(stored.method, PaymentMethod::Lightning);
    let Some(PaymentDetails::Lightning {
        invoice: settled_invoice,
        htlc_details,
        ..
    }) = &stored.details
    else {
        anyhow::bail!(
            "expected Lightning payment details, got {:?}",
            stored.details
        );
    };
    assert_eq!(settled_invoice, &invoice);
    assert!(
        htlc_details.is_none(),
        "a Spark-settled invoice has no HTLC"
    );

    Ok(())
}
