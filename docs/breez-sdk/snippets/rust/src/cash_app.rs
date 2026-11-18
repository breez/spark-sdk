use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn bridge_from_cash_app(client: &BridgingClient) -> Result<()> {
    // ANCHOR: bridge-from-cash-app
    // List the routes to the recipient's external-chain address
    // (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
    // Lightning, e.g. USDC on Base.
    let recipient = "<recipient address>".to_string();
    let routes = client
        .get_routes(GetBridgeRoutesRequest {
            recipient: recipient.clone(),
        })
        .await?
        .routes;
    let route = routes
        .into_iter()
        .find(|r| {
            r.source.chain == "lightning"
                && r.destination.asset == "USDC"
                && r.destination.chain == "base"
        })
        .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

    // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
    // USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
    let response = client
        .bridge_from_cash_app(CreateBridgeRequest {
            recipient,
            route,
            amount: 10_000_000,
            refund_address: None,
            fee_policy: None,
            max_slippage_bps: None,
        })
        .await?;

    // Open this Cash App link to pay. The recipient then receives the stablecoin.
    let info = response.info;
    info!("Open this URL in Cash App: {}", response.payment_request);
    info!(
        "Recipient receives ~{} {}",
        info.expected_received_amount, info.destination_asset
    );
    // ANCHOR_END: bridge-from-cash-app
    Ok(())
}

async fn bridge_to_cash_app(client: &BridgingClient) -> Result<()> {
    // ANCHOR: bridge-to-cash-app
    // List the routes to the Cash App user and pick what the payer pays in,
    // e.g. USDC on Base.
    let recipient = "$alice".to_string();
    let routes = client
        .get_routes(GetBridgeRoutesRequest {
            recipient: recipient.clone(),
        })
        .await?
        .routes;
    let route = routes
        .into_iter()
        .find(|r| r.source.asset == "USDC" && r.source.chain == "base")
        .ok_or_else(|| anyhow::anyhow!("No route from USDC on Base"))?;

    // Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
    // units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
    // refunded to the payer's address if delivery fails.
    let response = client
        .bridge_to_cash_app(CreateBridgeRequest {
            recipient,
            route,
            amount: 10_000_000,
            refund_address: Some("<payer address>".to_string()),
            fee_policy: None,
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
