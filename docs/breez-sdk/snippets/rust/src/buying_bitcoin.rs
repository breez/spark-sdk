use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn buy_bitcoin(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin
    // Optionally, prefill the purchase amount
    let optional_amount_sat = Some(100_000);
    // Optionally, set a redirect URL for after the purchase is completed
    let optional_redirect_url = Some("https://example.com/purchase-complete".to_string());

    let request = BuyBitcoinRequest::Moonpay {
        delivery: Some(MoonpayDelivery::Bitcoin {
            amount_sat: optional_amount_sat,
        }),
        redirect_url: optional_redirect_url,
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin
    Ok(())
}

async fn buy_bitcoin_via_cross_chain(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-cross-chain
    // USD amount to receive, in 6-decimal base units ($50)
    let amount = 50_000_000;

    let request = BuyBitcoinRequest::Moonpay {
        delivery: Some(MoonpayDelivery::CrossChain {
            amount,
            fee_mode: None,
        }),
        redirect_url: None,
    };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in a browser to complete the purchase:");
    info!("{}", response.url);

    if let Some(info) = response.cross_chain_info {
        info!("USDC to buy: {}", info.deposit_amount);
        info!(
            "Expected to receive: {} {}",
            info.expected_received_amount, info.destination_asset
        );
        info!("Conversion fee: {}", info.service_fee_amount);
    }
    // ANCHOR_END: buy-bitcoin-cross-chain
    Ok(())
}

async fn buy_bitcoin_via_cashapp(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: buy-bitcoin-cashapp
    // Cash App requires the amount to be specified up front.
    let amount_sats = 50_000;

    let request = BuyBitcoinRequest::CashApp { amount_sats };

    let response = sdk.buy_bitcoin(request).await?;
    info!("Open this URL in Cash App to complete the purchase:");
    info!("{}", response.url);
    // ANCHOR_END: buy-bitcoin-cashapp
    Ok(())
}
