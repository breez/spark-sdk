use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn buy_bitcoin_basic(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-basic
    // Buy Bitcoin with funds deposited directly into the user's wallet
    let request = BuyBitcoinRequest::default();

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-basic
    Ok(())
}

async fn buy_bitcoin_with_amount(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-with-amount
    // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
    let request = BuyBitcoinRequest {
        locked_amount_sat: Some(100_000),
        ..Default::default()
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-with-amount
    Ok(())
}

async fn buy_bitcoin_with_redirect(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-with-redirect
    // Provide a custom redirect URL for after the purchase
    let request = BuyBitcoinRequest {
        locked_amount_sat: Some(100_000),
        redirect_url: Some("https://example.com/purchase-complete".to_string()),
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-with-redirect
    Ok(())
}

