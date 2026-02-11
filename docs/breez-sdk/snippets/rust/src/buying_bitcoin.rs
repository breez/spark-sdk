use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn buy_bitcoin(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin
    // Optionally, lock the purchase to a specific amount
    let optional_locked_amount_sat = Some(100_000);
    // Optionally, set a redirect URL for after the purchase is completed
    let optional_redirect_url = Some("https://example.com/purchase-complete".to_string());

    let request = BuyBitcoinRequest {
        locked_amount_sat: optional_locked_amount_sat,
        redirect_url: optional_redirect_url,
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin
    Ok(())
}
