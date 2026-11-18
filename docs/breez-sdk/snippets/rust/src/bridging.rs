use std::sync::Arc;

use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn new_client() -> Result<Arc<BridgingClient>> {
    // ANCHOR: new-bridging-client
    let config = default_bridging_config("<your api key>".to_string());
    let client = new_bridging_client(config).await?;
    // ANCHOR_END: new-bridging-client
    Ok(client)
}

fn client_from_sdk(sdk: &BreezSdk) -> Result<Arc<BridgingClient>> {
    // ANCHOR: get-bridging-client
    let client = sdk.get_bridging_client()?;
    // ANCHOR_END: get-bridging-client
    Ok(client)
}

async fn create_bridge(client: &BridgingClient) -> Result<()> {
    // ANCHOR: create-bridge
    // Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
    // address.
    let recipient = "<recipient>".to_string();

    // List the routes that pay them and pick what the payer pays in, e.g.
    // USDC on Base.
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

    // Ask for $10. The amount is in the recipient's stablecoin when they are
    // paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
    // refunded to the payer's address if the recipient can't be paid.
    let response = client
        .create_bridge(CreateBridgeRequest {
            recipient,
            route,
            amount: 10_000_000,
            // Required to pay a Lightning address.
            refund_address: Some("<payer address on Base>".to_string()),
            fee_policy: None,
            max_slippage_bps: None,
        })
        .await?;

    // Show the payer what to pay, and store the bridge id and status token to
    // look up the status later.
    info!("Pay with: {}", response.payment_request);
    info!("Bridge id: {}", response.bridge_id);
    // ANCHOR_END: create-bridge
    Ok(())
}

async fn get_bridge_status(
    client: &BridgingClient,
    bridge_id: String,
    status_token: Option<String>,
) -> Result<()> {
    // ANCHOR: get-bridge-status
    let response = client
        .get_bridge_status(GetBridgeStatusRequest {
            bridge_id,
            status_token,
        })
        .await?;
    match response.status {
        BridgeStatus::Completed => info!("Delivered {:?}", response.amount_out),
        BridgeStatus::Refunded | BridgeStatus::Failed | BridgeStatus::Expired => {
            info!("Not delivered: {:?}", response.status)
        }
        _ => info!("Not final yet: {:?}", response.status),
    }
    // ANCHOR_END: get-bridge-status
    Ok(())
}
