use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn fetch_token_balances(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: fetch-token-balances
    let info = sdk
        .get_info(GetInfoRequest {
            // ensure_synced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            ensure_synced: Some(false),
        })
        .await?;

    // Token balances are a map of token identifier to balance
    let token_balances = info.token_balances;
    for (token_id, token_balance) in token_balances {
        info!("Token ID: {}", token_id);
        info!("Balance: {}", token_balance.balance);
        info!("Name: {}", token_balance.token_metadata.name);
        info!("Ticker: {}", token_balance.token_metadata.ticker);
        info!("Decimals: {}", token_balance.token_metadata.decimals);
    }
    // ANCHOR_END: fetch-token-balances
    Ok(())
}

async fn fetch_token_metadata(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: fetch-token-metadata
    let response = sdk
        .get_tokens_metadata(GetTokensMetadataRequest {
            token_identifiers: vec![
                String::from("<token identifier 1>"),
                String::from("<token identifier 2>"),
            ],
        })
        .await?;

    let tokens_metadata = response.tokens_metadata;
    for token_metadata in tokens_metadata {
        info!("Token ID: {}", token_metadata.identifier);
        info!("Name: {}", token_metadata.name);
        info!("Ticker: {}", token_metadata.ticker);
        info!("Decimals: {}", token_metadata.decimals);
        info!("Max Supply: {}", token_metadata.max_supply);
        info!("Is Freezable: {}", token_metadata.is_freezable);
    }
    // ANCHOR_END: fetch-token-metadata
    Ok(())
}

async fn receive_token_payment_spark_invoice(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: receive-token-payment-spark-invoice
    let token_identifier = Some("<token identifier>".to_string());
    let optional_description = Some("<invoice description>".to_string());
    let optional_amount = Some(5_000);
    // Optionally set the expiry UNIX timestamp in seconds
    let optional_expiry_time_seconds = Some(1716691200);
    let optional_sender_public_key = Some("<sender public key>".to_string());

    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                token_identifier,
                description: optional_description,
                amount: optional_amount,
                expiry_time: optional_expiry_time_seconds,
                sender_public_key: optional_sender_public_key,
            },
        })
        .await?;

    let payment_request = response.payment_request;
    info!("Payment request: {payment_request}");
    let receive_fee = response.fee;
    info!("Fees: {receive_fee} token base units");
    // ANCHOR_END: receive-token-payment-spark-invoice
    Ok(())
}

async fn send_token_payment(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: send-token-payment
    let payment_request = "<spark address or invoice>".to_string();
    // Token identifier must match the invoice in case it specifies one.
    let token_identifier = Some("<token identifier>".to_string());
    // Set the amount of tokens you wish to send.
    let optional_amount = Some(1_000);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: optional_amount,
            token_identifier,
            conversion_options: None,
        })
        .await?;

    // If the fees are acceptable, continue to send the token payment
    match &prepare_response.payment_method {
        SendPaymentMethod::SparkAddress {
            fee,
            token_identifier: token_id,
            ..
        } => {
            info!("Token ID: {:?}", token_id);
            info!("Fees: {} token base units", fee);
        }
        SendPaymentMethod::SparkInvoice {
            fee,
            token_identifier: token_id,
            ..
        } => {
            info!("Token ID: {:?}", token_id);
            info!("Fees: {} token base units", fee);
        }
        _ => {}
    }

    // Send the token payment
    let send_response = sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await?;
    let payment = send_response.payment;
    info!("Payment: {payment:?}");
    // ANCHOR_END: send-token-payment
    Ok(())
}

async fn fetch_conversion_limits(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: fetch-token-conversion-limits
    // Fetch limits for converting Bitcoin to a token
    let response = sdk
        .fetch_conversion_limits(FetchConversionLimitsRequest {
            conversion_type: ConversionType::FromBitcoin,
            token_identifier: Some("<token identifier>".to_string()),
        })
        .await?;

    if let Some(min_from) = response.min_from_amount {
        info!("Minimum BTC to convert: {} sats", min_from);
    }
    if let Some(min_to) = response.min_to_amount {
        info!("Minimum tokens to receive: {} base units", min_to);
    }

    // Fetch limits for converting a token to Bitcoin
    let response = sdk
        .fetch_conversion_limits(FetchConversionLimitsRequest {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "<token identifier>".to_string(),
            },
            token_identifier: None,
        })
        .await?;

    if let Some(min_from) = response.min_from_amount {
        info!("Minimum tokens to convert: {} base units", min_from);
    }
    if let Some(min_to) = response.min_to_amount {
        info!("Minimum BTC to receive: {} sats", min_to);
    }
    // ANCHOR_END: fetch-token-conversion-limits
    Ok(())
}

async fn prepare_send_payment_token_conversion(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-send-payment-token-conversion
    let payment_request = "<spark address or invoice>".to_string();
    // Token identifier must match the invoice in case it specifies one.
    let token_identifier = Some("<token identifier>".to_string());
    // Set the amount of tokens you wish to send.
    let optional_amount = Some(1_000);
    // Set to use Bitcoin funds to pay via token conversion
    let optional_max_slippage_bps = Some(50);
    let optional_completion_timeout_secs = Some(30);
    let conversion_options = Some(ConversionOptions {
        conversion_type: ConversionType::FromBitcoin,
        max_slippage_bps: optional_max_slippage_bps,
        completion_timeout_secs: optional_completion_timeout_secs,
    });

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount: optional_amount,
            token_identifier,
            conversion_options,
        })
        .await?;

    // If the fees are acceptable, continue to send the token payment
    if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
        info!("Estimated conversion amount: {} sats", conversion_estimate.amount);
        info!("Estimated conversion fee: {} sats", conversion_estimate.fee);
    }
    // ANCHOR_END: prepare-send-payment-token-conversion
    Ok(())
}