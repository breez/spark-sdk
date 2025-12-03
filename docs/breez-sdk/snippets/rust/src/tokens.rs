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

async fn prepare_transfer_token_to_bitcoin(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-transfer-token-to-bitcoin
    let token_identifier = "<token identifier>".to_string();
    // Amount in token base units
    let amount = 10_000_000;

    let prepare_response = sdk
        .prepare_transfer_token(PrepareTransferTokenRequest {
            transfer_type: TransferType::ToBitcoin,
            token_identifier,
            amount,
        })
        .await?;

    let estimated_receive_amount = prepare_response.estimated_receive_amount;
    let fee = prepare_response.fee;
    info!("Estimated receive amount: {estimated_receive_amount} sats");
    info!("Fees: {fee} token base units");
    // ANCHOR_END: prepare-transfer-token-to-bitcoin
    Ok(())
}

async fn prepare_transfer_token_from_bitcoin(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-transfer-token-from-bitcoin
    let token_identifier = "<token identifier>".to_string();
    // Amount in satoshis
    let amount = 10_000;

    let prepare_response = sdk
        .prepare_transfer_token(PrepareTransferTokenRequest {
            transfer_type: TransferType::FromBitcoin,
            token_identifier,
            amount,
        })
        .await?;

    let estimated_receive_amount = prepare_response.estimated_receive_amount;
    let fee = prepare_response.fee;
    info!("Estimated receive amount: {estimated_receive_amount} token base units");
    info!("Fees: {fee} sats");
    // ANCHOR_END: prepare-transfer-token-from-bitcoin
    Ok(())
}

async fn transfer_token(
    sdk: &BreezSdk,
    prepare_response: PrepareTransferTokenResponse,
) -> Result<()> {
    // ANCHOR: transfer-token
    // Set the maximum slippage to 1% in basis points
    let optional_max_slippage_bps = 100;

    let response = sdk
        .transfer_token(TransferTokenRequest {
            prepare_response,
            max_slippage_bps: Some(optional_max_slippage_bps),
        })
        .await?;

    let sent_payment = response.sent_payment;
    let received_payment = response.received_payment;
    info!("Sent payment: {sent_payment:?}");
    info!("Received payment: {received_payment:?}");
    // ANCHOR_END: transfer-token
    Ok(())
}
