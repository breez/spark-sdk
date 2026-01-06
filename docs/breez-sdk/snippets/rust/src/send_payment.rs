use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn prepare_send_payment_lightning_bolt11(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-lightning-bolt11
    let payment_request = "<bolt11 invoice>".to_string();
    // Optionally set the amount you wish the pay the receiver
    let optional_amount_sats = Some(5_000);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: optional_amount_sats,
            token_identifier: None,
            token_conversion_options: None,
        })
        .await?;

    // If the fees are acceptable, continue to create the Send Payment
    if let SendPaymentMethod::Bolt11Invoice {
        spark_transfer_fee_sats,
        lightning_fee_sats,
        ..
    } = prepare_response.payment_method
    {
        // Fees to pay via Lightning
        info!("Lightning Fees: {lightning_fee_sats} sats");
        // Or fees to pay (if available) via a Spark transfer
        info!("Spark Transfer Fees: {spark_transfer_fee_sats:?} sats");
    }
    // ANCHOR_END: prepare-send-payment-lightning-bolt11
    Ok(())
}

async fn prepare_send_payment_lightning_onchain(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-onchain
    let payment_request = "<bitcoin address>".to_string();
    // Set the amount you wish the pay the receiver
    let amount_sats = Some(50_000);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: amount_sats,
            token_identifier: None,
            token_conversion_options: None,
        })
        .await?;

    // If the fees are acceptable, continue to create the Send Payment
    if let SendPaymentMethod::BitcoinAddress { fee_quote, .. } = &prepare_response.payment_method {
        info!("Slow Fees: {} sats", fee_quote.speed_slow.total_fee_sat());
        info!(
            "Medium Fees: {} sats",
            fee_quote.speed_medium.total_fee_sat()
        );
        info!("Fast Fees: {} sats", fee_quote.speed_fast.total_fee_sat());
    }
    // ANCHOR_END: prepare-send-payment-onchain
    Ok(())
}

async fn prepare_send_payment_spark_address(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-spark-address
    let payment_request = "<spark address>".to_string();
    // Set the amount you wish the pay the receiver
    let amount_sats = Some(50_000);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: amount_sats,
            token_identifier: None,
            token_conversion_options: None,
        })
        .await?;

    // If the fees are acceptable, continue to create the Send Payment
    if let SendPaymentMethod::SparkAddress { fee, .. } = prepare_response.payment_method {
        info!("Fees: {} sats", fee);
    }
    // ANCHOR_END: prepare-send-payment-spark-address
    Ok(())
}

async fn prepare_send_payment_spark_invoice(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-spark-invoice
    let payment_request = "<spark invoice>".to_string();
    // Optionally set the amount you wish the pay the receiver
    let optional_amount_sats = Some(50_000);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: optional_amount_sats,
            token_identifier: None,
            token_conversion_options: None,
        })
        .await?;

    // If the fees are acceptable, continue to create the Send Payment
    if let SendPaymentMethod::SparkInvoice { fee, .. } = prepare_response.payment_method {
        info!("Fees: {} sats", fee);
    }
    // ANCHOR_END: prepare-send-payment-spark-invoice
    Ok(())
}

async fn prepare_send_payment_token_conversion(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-token-conversion
    let payment_request = "<payment request>".to_string();
    // Set to use token funds to pay via token conversion
    let optional_max_slippage_bps = Some(50);
    let optional_completion_timeout_secs = Some(30);
    let token_conversion_options = Some(TokenConversionOptions {
        conversion_type: TokenConversionType::ToBitcoin {
            from_token_identifier: "<token identifier>".to_string(),
        },
        max_slippage_bps: optional_max_slippage_bps,
        completion_timeout_secs: optional_completion_timeout_secs,
    });

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: None,
            token_identifier: None,
            token_conversion_options,
        })
        .await?;

    // If the fees are acceptable, continue to create the Send Payment
    if let Some(token_conversion_fee) = prepare_response.token_conversion_fee {
        info!("Estimated token conversion fee: {token_conversion_fee} token base units");
    }
    // ANCHOR_END: prepare-send-payment-token-conversion
    Ok(())
}

async fn send_payment_lightning_bolt11(
    sdk: &BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<()> {
    // ANCHOR: send-payment-lightning-bolt11
    let options = Some(SendPaymentOptions::Bolt11Invoice {
        prefer_spark: false,
        completion_timeout_secs: Some(10),
    });
    let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
    let send_response = sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options,
            idempotency_key: optional_idempotency_key,
        })
        .await?;
    let payment = send_response.payment;
    info!("Payment: {payment:?}");
    // ANCHOR_END: send-payment-lightning-bolt11
    Ok(())
}

async fn send_payment_onchain(
    sdk: &BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<()> {
    // ANCHOR: send-payment-onchain
    let options = SendPaymentOptions::BitcoinAddress {
        confirmation_speed: OnchainConfirmationSpeed::Medium,
    };
    let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
    let send_response = sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: Some(options),
            idempotency_key: optional_idempotency_key,
        })
        .await?;
    let payment = send_response.payment;
    info!("Payment: {payment:?}");
    // ANCHOR_END: send-payment-onchain
    Ok(())
}

async fn send_payment_spark(
    sdk: &BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<()> {
    // ANCHOR: send-payment-spark
    let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
    let send_response = sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: optional_idempotency_key,
        })
        .await?;
    let payment = send_response.payment;
    info!("Payment: {payment:?}");
    // ANCHOR_END: send-payment-spark
    Ok(())
}
