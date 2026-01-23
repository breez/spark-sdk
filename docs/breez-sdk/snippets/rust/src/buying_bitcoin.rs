use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn buy_bitcoin_basic(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-basic
    let request = BuyBitcoinRequest {
        address: "bc1qexample...".to_string(), // Your Bitcoin address
        locked_amount_sat: None,
        max_amount_sat: None,
        redirect_url: None,
    };

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
        address: "bc1qexample...".to_string(),
        locked_amount_sat: Some(100_000), // Pre-fill with 100,000 sats
        max_amount_sat: None,
        redirect_url: None,
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-with-amount
    Ok(())
}

async fn buy_bitcoin_with_limits(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-with-limits
    // Set both a locked amount and maximum amount
    let request = BuyBitcoinRequest {
        address: "bc1qexample...".to_string(),
        locked_amount_sat: Some(50_000),   // Pre-fill with 50,000 sats
        max_amount_sat: Some(500_000),     // Limit to 500,000 sats max
        redirect_url: None,
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-with-limits
    Ok(())
}

async fn buy_bitcoin_with_redirect(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-with-redirect
    // Provide a custom redirect URL for after the purchase
    let request = BuyBitcoinRequest {
        address: "bc1qexample...".to_string(),
        locked_amount_sat: Some(100_000),
        max_amount_sat: None,
        redirect_url: Some("https://example.com/purchase-complete".to_string()),
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-with-redirect
    Ok(())
}
