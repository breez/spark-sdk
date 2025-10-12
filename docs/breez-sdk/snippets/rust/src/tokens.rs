use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn fetch_token_balances(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: fetch-token-balances
    let info = sdk.get_info(GetInfoRequest {
      // ensure_synced: true will ensure the SDK is synced with the Spark network
      // before returning the balance
      ensure_synced: Some(false),
    }).await?;
    
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

async fn send_token_payment(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: send-token-payment
    let payment_request = "<spark address>".to_string();
    let token_identifier = Some("<token identifier>".to_string());
    // Set the amount of tokens you wish to send
    let amount = Some(1_000);
    
    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request,
            amount,
            token_identifier,
        })
        .await?;

    // If the fees are acceptable, continue to send the token payment
    if let SendPaymentMethod::SparkAddress { 
        fee,
        token_identifier: token_id,
        .. 
    } = &prepare_response.payment_method {
        info!("Token ID: {:?}", token_id);
        info!("Fees: {} token units", fee);
    }

    // Send the token payment
    let send_response = sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
        })
        .await?;
    let payment = send_response.payment;
    info!("Payment: {payment:?}");
    // ANCHOR_END: send-token-payment
    Ok(())
}
