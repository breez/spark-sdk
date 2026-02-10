use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn buy_bitcoin(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin
    // Buy Bitcoin with funds deposited directly into the user's wallet.
    // Optionally lock the purchase to a specific amount and provide a redirect URL.
    let request = BuyBitcoinRequest {
        locked_amount_sat: Some(100_000),
        redirect_url: Some("https://example.com/purchase-complete".to_string()),
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin
    Ok(())
}
