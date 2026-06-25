use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn get_cross_chain_routes(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: cross-chain-get-routes
    let input = "<recipient address>";
    let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
        anyhow::bail!("Not a cross-chain address");
    };

    let routes = sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::Send {
            address_details: address_details.clone(),
        })
        .await?;

    for route in &routes {
        info!(
            "Route via {:?}: {}/{}",
            route.provider, route.chain, route.asset
        );
    }
    // ANCHOR_END: cross-chain-get-routes
    Ok(())
}

async fn prepare_send_payment_cross_chain(
    sdk: &BreezSdk,
    address_details: CrossChainAddressDetails,
    route: CrossChainRoutePair,
) -> Result<()> {
    // ANCHOR: cross-chain-prepare
    // Optionally set the maximum slippage in basis points (10 to 500)
    let optional_max_slippage_bps = Some(100);

    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::CrossChain {
                address: address_details.address.clone(),
                route,
                max_slippage_bps: optional_max_slippage_bps,
                target_overpay_bps: None,
            },
            amount: Some(50_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    if let SendPaymentMethod::CrossChainAddress {
        amount_in,
        estimated_out,
        fee_amount,
        expires_at,
        ..
    } = &prepare_response.payment_method
    {
        info!("Amount in: {amount_in}");
        info!("Estimated out: {estimated_out}");
        info!("Provider fee: {fee_amount}");
        info!("Quote expires at: {expires_at}");
    }
    // ANCHOR_END: cross-chain-prepare
    Ok(())
}

async fn send_payment_cross_chain(
    sdk: &BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<()> {
    // ANCHOR: cross-chain-send
    // Only valid for sends with no token leg (see Retry safety).
    let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
    let send_response = sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: optional_idempotency_key,
        })
        .await?;
    let payment = send_response.payment;
    info!("Payment: {payment:?}");
    // ANCHOR_END: cross-chain-send
    Ok(())
}

async fn get_cross_chain_receive_routes(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: cross-chain-get-receive-routes
    let routes = sdk
        .get_cross_chain_routes(&CrossChainRouteFilter::Receive {
            contract_address: None,
        })
        .await?;

    for route in &routes {
        info!(
            "Route via {:?}: {}/{} -> Spark",
            route.provider, route.chain, route.asset
        );
    }
    // ANCHOR_END: cross-chain-get-receive-routes
    Ok(())
}

async fn receive_payment_cross_chain(sdk: &BreezSdk, route: CrossChainRoutePair) -> Result<()> {
    // ANCHOR: cross-chain-receive
    // amount is in source-asset base units (e.g. USDC base units when source is USDC)
    let amount = 1_000_000u128;
    // Optionally set the destination Spark-side asset. None = auto: active
    // stable-balance token if the route supports it, otherwise BTC.
    let optional_destination: Option<SparkAsset> = None;
    // Optionally set the maximum slippage in basis points (10 to 500)
    let optional_max_slippage_bps = Some(100);

    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::CrossChain {
                route,
                amount,
                destination: optional_destination,
                max_slippage_bps: optional_max_slippage_bps,
            },
        })
        .await?;

    info!("Payment request: {}", response.payment_request);
    if let Some(info) = response.cross_chain_info {
        info!("Deposit address: {}", info.deposit_address);
        info!("Deposit amount: {}", info.deposit_amount);
        let denom = if info.token_identifier.is_some() { "USDB" } else { "BTC" };
        info!(
            "Expected received: {} {denom}",
            info.expected_received_amount
        );
        info!("Expires at: {}", info.expires_at);
    }
    // ANCHOR_END: cross-chain-receive
    Ok(())
}
