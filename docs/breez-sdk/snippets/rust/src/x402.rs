use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

fn get_x402(sdk: &BreezSdk) -> X402 {
    // ANCHOR: get-x402
    let x402 = sdk.get_x402();
    // ANCHOR_END: get-x402
    x402
}

async fn pay_http_request(x402: &X402, payment_required_header: String) -> Result<()> {
    // ANCHOR: pay-http-request
    // The request that returned 402, exactly as it will be sent again
    let request = X402Request::Http {
        method: "GET".to_string(),
        url: "https://api.example.com/article/A".to_string(),
        headers: vec![X402Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
    };
    let response = x402
        .pay(PayX402Request {
            // The PAYMENT-REQUIRED header of the 402 response
            payment_required: payment_required_header,
            request,
            max_amount: X402PaymentAmount::Lightning { amount_sats: 100 },
            conversion_options: None,
            completion_timeout_secs: None,
        })
        .await?;

    if let Some(_payment_signature) = response.payment_signature {
        // Send the request again with it as the PAYMENT-SIGNATURE header.
        // It proves the payment, so don't log it.
    } else {
        // Still in flight: paying the same challenge again picks it up
        info!("Payment status: {:?}", response.status);
    }
    // ANCHOR_END: pay-http-request
    Ok(())
}

async fn pay_mcp_tool_call(x402: &X402, structured_content: String) -> Result<()> {
    // ANCHOR: pay-mcp-tool-call
    let request = X402Request::Mcp {
        server: "https://api.example.com/mcp".to_string(),
        // The tools/call params, exactly as they will be sent again
        params: r#"{"name":"get_article","arguments":{"article":"A"}}"#.to_string(),
    };
    let response = x402
        .pay(PayX402Request {
            // The structuredContent of the tool result
            payment_required: structured_content,
            request,
            max_amount: X402PaymentAmount::Lightning { amount_sats: 100 },
            conversion_options: None,
            completion_timeout_secs: None,
        })
        .await?;

    if let Some(_payment_signature) = response.payment_signature {
        // Call the tool again with this JSON in params._meta["x402/payment"]
    }
    // ANCHOR_END: pay-mcp-tool-call
    Ok(())
}

async fn create_challenge(x402: &X402) -> Result<()> {
    // ANCHOR: create-challenge
    // The unpaid request, with its URL built from your public origin
    let request = X402Request::Http {
        method: "GET".to_string(),
        url: "https://api.example.com/article/A".to_string(),
        headers: vec![X402Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
    };
    let response = x402
        .create_challenge(CreateX402ChallengeRequest {
            request,
            // Headers that change what is served, so the payment covers them too
            bound_names: vec!["accept".to_string()],
            amounts: vec![X402PaymentAmount::Lightning { amount_sats: 21 }],
            max_timeout_secs: None,
            description: Some("An article".to_string()),
            mime_type: Some("application/json".to_string()),
        })
        .await?;

    // Respond with status 402 and this PAYMENT-REQUIRED header
    info!("PAYMENT-REQUIRED: {}", response.payment_required);
    // ANCHOR_END: create-challenge
    Ok(())
}

async fn settle_payment(x402: &X402, payment_signature_header: String) -> Result<()> {
    // ANCHOR: settle-payment
    // The paid request, exactly as received
    let request = X402Request::Http {
        method: "GET".to_string(),
        url: "https://api.example.com/article/A".to_string(),
        headers: vec![X402Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
    };
    let response = x402
        .settle(SettleX402Request {
            // The PAYMENT-SIGNATURE header of the request
            payment_signature: payment_signature_header,
            request,
            bound_names: vec!["accept".to_string()],
            // The same price and lifetime the challenge was created with
            amounts: vec![X402PaymentAmount::Lightning { amount_sats: 21 }],
            max_timeout_secs: None,
        })
        .await?;

    // Send the PAYMENT-RESPONSE header whatever the status
    info!("PAYMENT-RESPONSE: {}", response.payment_response);
    match response.status {
        X402SettleStatus::Settled => {
            // Serve the resource
        }
        X402SettleStatus::Pending => {
            // Respond with status 402 and no new challenge
        }
        X402SettleStatus::Malformed => {
            // Respond with status 400
        }
        X402SettleStatus::Refused => {
            // Respond with status 402 and a new challenge
        }
        X402SettleStatus::Error => {
            // Respond with status 500
        }
    }
    // ANCHOR_END: settle-payment
    Ok(())
}
