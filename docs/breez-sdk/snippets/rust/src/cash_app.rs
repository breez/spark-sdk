use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn bridge_from_cash_app(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: bridge-from-cash-app
    // Parse the recipient's external-chain address (EVM/Solana/Tron).
    let input = "<recipient address>";
    let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
        anyhow::bail!("Not a cross-chain address");
    };

    // List the stablecoin destinations Cash App can fund over Lightning and
    // pick one, e.g. USDC on Base.
    let routes = sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::Send {
            address_details: address_details.clone(),
            delivery_method: Some(DeliveryMethod::Lightning),
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
        .bridge_from_cash_app(BridgeFromCashAppRequest {
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
    // ANCHOR_END: bridge-from-cash-app
    Ok(())
}

async fn bridge_to_cash_app(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: bridge-to-cash-app
    // List the stablecoin sources that can pay a Cash App user over Lightning
    // and pick one, e.g. USDC on Base.
    let routes = sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::Receive {
            contract_address: None,
            delivery_method: Some(DeliveryMethod::Lightning),
        })
        .await?;
    let route = routes
        .into_iter()
        .find(|r| r.asset == "USDC" && r.chain == "base")
        .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

    // Pay $10 of USDC to the Cash App user $alice. The amount is in the route
    // asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC, about $10.
    // The deposit is refunded to the payer's address if delivery fails.
    let response = sdk
        .bridge_to_cash_app(BridgeToCashAppRequest {
            recipient: "$alice".to_string(),
            route,
            amount: 10_000_000,
            fee_policy: None,
            refund_address: "<payer address>".to_string(),
            max_slippage_bps: None,
        })
        .await?;

    // Show the payer what to pay. The recipient then receives Bitcoin.
    let info = response.info;
    info!("Pay with: {}", response.payment_request);
    info!(
        "Deposit {} to {}, recipient receives ~{} sats",
        info.deposit_amount, info.deposit_address, info.expected_received_amount
    );
    // ANCHOR_END: bridge-to-cash-app
    Ok(())
}
