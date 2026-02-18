use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn send_htlc_payment(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: send-htlc-payment
    let payment_request = "<spark address>".to_string();
    // Set the amount you wish to pay the receiver
    let amount_sats = Some(50_000);
    let prepare_request = PrepareSendPaymentRequest {
        payment_request,
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    };
    let prepare_response = sdk.prepare_send_payment(prepare_request).await?;

    // If the fees are acceptable, continue to create the HTLC Payment
    if let SendPaymentMethod::SparkAddress { fee, .. } = prepare_response.payment_method {
        info!("Fees: {} sats", fee);
    }

    let preimage = "<32-byte unique preimage hex>";
    let preimage_bytes = hex::decode(preimage)?;
    let payment_hash_bytes = sha256::digest(preimage_bytes);
    let payment_hash = hex::encode(payment_hash_bytes);

    // Set the HTLC options
    let options = SendPaymentOptions::SparkAddress {
        htlc_options: Some(SparkHtlcOptions {
            payment_hash,
            expiry_duration_secs: 1000,
        }),
    };

    let request = SendPaymentRequest {
        prepare_response,
        options: Some(options),
        idempotency_key: None,
    };
    let send_response = sdk.send_payment(request).await?;
    let payment = send_response.payment;
    // ANCHOR_END: send-htlc-payment
    Ok(())
}

async fn receive_hodl_invoice_payment(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-hodl-invoice-payment
    let preimage = "<32-byte unique preimage hex>";
    let preimage_bytes = hex::decode(preimage)?;
    let payment_hash_bytes = sha256::digest(preimage_bytes);
    let payment_hash = hex::encode(payment_hash_bytes);

    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "HODL invoice".to_string(),
                amount_sats: Some(50_000),
                expiry_secs: None,
                payment_hash: Some(payment_hash),
            },
        })
        .await?;

    let invoice = response.payment_request;
    info!("HODL invoice: {invoice}");
    // ANCHOR_END: receive-hodl-invoice-payment
    Ok(())
}

async fn list_claimable_htlc_payments(sdk: &BreezSdk) -> Result<Vec<Payment>> {
    // ANCHOR: list-claimable-htlc-payments
    let request = ListPaymentsRequest {
        type_filter: Some(vec![PaymentType::Receive]),
        status_filter: Some(vec![PaymentStatus::Pending]),
        payment_details_filter: Some(vec![
            PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
                conversion_refund_needed: None,
            },
            PaymentDetailsFilter::Lightning {
                htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
            },
        ]),
        ..Default::default()
    };

    let response = sdk.list_payments(request).await?;
    let payments = response.payments;

    for payment in &payments {
        match &payment.details {
            Some(PaymentDetails::Spark {
                htlc_details: Some(htlc),
                ..
            }) => {
                info!("Spark HTLC expiry time: {}", htlc.expiry_time);
            }
            Some(PaymentDetails::Lightning {
                htlc_details: htlc,
                ..
            }) => {
                info!("Lightning HTLC expiry time: {}", htlc.expiry_time);
            }
            _ => {}
        }
    }
    // ANCHOR_END: list-claimable-htlc-payments
    Ok(payments)
}

async fn claim_htlc_payment(sdk: &BreezSdk) -> Result<Payment> {
    // ANCHOR: claim-htlc-payment
    let preimage = "<preimage hex>".to_string();
    let response = sdk
        .claim_htlc_payment(ClaimHtlcPaymentRequest { preimage })
        .await?;
    let payment = response.payment;
    // ANCHOR_END: claim-htlc-payment
    Ok(payment)
}
