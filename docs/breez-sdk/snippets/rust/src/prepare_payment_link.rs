use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn prepare_payment_link_via_cashapp(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-payment-link-cashapp
    // Parse the recipient's external-chain address (EVM/Solana/Tron).
    let input = "<recipient address>";
    let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
        anyhow::bail!("Not a cross-chain address");
    };

    // List the stablecoin destinations you can send to and pick one, e.g. USDC
    // on Base. Payment-link routes are funded by Cash App over Lightning.
    let routes = sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::PaymentLink {
            address_details: address_details.clone(),
        })
        .await?;
    let route = routes
        .into_iter()
        .find(|r| r.asset == "USDC" && r.chain == "base")
        .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

    // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
    // route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
    // about $10.
    let response = sdk
        .prepare_payment_link(PreparePaymentLinkRequest {
            address: address_details.address,
            route,
            amount: 10_000_000,
            fee_policy: None,
            max_slippage_bps: None,
        })
        .await?;

    // Open this Cash App URL to pay. The recipient then receives the stablecoin.
    info!("Open this URL in Cash App: {}", response.url);
    info!(
        "Recipient receives ~{} {}",
        response.estimated_out, response.asset
    );
    // ANCHOR_END: prepare-payment-link-cashapp
    Ok(())
}
