//! A Bolt11 invoice that advertises a Spark destination, paid over Spark.
//!
//! The point of the fallback is that both ends can still tell the payment
//! settled the invoice that was issued, even though nothing reached the SSP.

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

/// Alice pays Bob's Bolt11 over Spark. Both of them see the invoice, not a
/// bare Spark transfer.
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

    let sent = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?
        .payment;

    // Alice paid a Bolt11, so that is what her payment reports, even though a
    // transfer is what carried it.
    assert_bolt11_settled_over_spark(&sent, &invoice)?;

    // Surviving a sync is the point of recording the link: the sync rebuilds
    // the payment from a transfer that does not name the Bolt11.
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_stored = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: sent.id.clone(),
        })
        .await?
        .payment;
    assert_bolt11_settled_over_spark(&alice_stored, &invoice)?;

    // Bob's payment must report as the Bolt11 being paid, not as a bare Spark
    // receive, and must carry no HTLC: none was ever created.
    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received.amount, u128::from(AMOUNT_SATS));
    assert_bolt11_settled_over_spark(&received, &invoice)?;

    // Read back as well as trusting the event: whichever ingestion path sees
    // the transfer first, storage is what a caller lists and waits on.
    let stored = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: received.id.clone(),
        })
        .await?
        .payment;
    assert_bolt11_settled_over_spark(&stored, &invoice)?;

    Ok(())
}

/// Asserts `payment` reports as `invoice` being paid over Spark.
fn assert_bolt11_settled_over_spark(payment: &Payment, invoice: &str) -> Result<()> {
    assert_eq!(payment.method, PaymentMethod::Lightning);
    let Some(PaymentDetails::Lightning {
        invoice: settled_invoice,
        htlc_details,
        ..
    }) = &payment.details
    else {
        anyhow::bail!(
            "expected Lightning payment details, got {:?}",
            payment.details
        );
    };
    assert_eq!(settled_invoice, invoice);
    assert!(
        htlc_details.is_none(),
        "a Spark-settled invoice has no HTLC"
    );
    Ok(())
}
