//! x402 payments over Lightning between two wallets: one charging for an HTTP
//! resource, one paying for it.

use std::time::Duration;

use anyhow::{Result, ensure};
use base64::{Engine, prelude::BASE64_STANDARD};
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

const URL_A: &str = "https://api.example.com/article/A";
const URL_B: &str = "https://api.example.com/article/B";
const PRICE_SATS: u64 = 1_000;

fn get(url: &str) -> X402Request {
    X402Request::Http {
        method: "GET".to_string(),
        url: url.to_string(),
        headers: vec![X402Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
    }
}

async fn challenge(server: &SdkInstance, url: &str) -> Result<CreateX402ChallengeResponse> {
    Ok(server
        .sdk
        .get_x402()
        .create_challenge(CreateX402ChallengeRequest {
            request: get(url),
            bound_names: vec!["accept".to_string()],
            amounts: vec![X402PaymentAmount::Lightning {
                amount_sats: PRICE_SATS,
            }],
            max_timeout_secs: None,
            description: Some("An article".to_string()),
            mime_type: Some("application/json".to_string()),
        })
        .await?)
}

async fn pay(client: &SdkInstance, payment_required: &str, url: &str) -> Result<String> {
    let response = client
        .sdk
        .get_x402()
        .pay(PayX402Request {
            payment_required: payment_required.to_string(),
            request: get(url),
            max_amount: X402PaymentAmount::Lightning {
                amount_sats: PRICE_SATS * 2,
            },
            conversion_options: None,
            completion_timeout_secs: Some(60),
        })
        .await?;
    ensure!(
        response.status == X402PaymentStatus::Paid,
        "payment is {:?}",
        response.status
    );
    Ok(response
        .payment_signature
        .expect("a paid response has a signature"))
}

async fn settle(server: &SdkInstance, payment: &str, url: &str) -> Result<SettleX402Response> {
    Ok(server
        .sdk
        .get_x402()
        .settle(SettleX402Request {
            payment: payment.to_string(),
            request: get(url),
            bound_names: vec!["accept".to_string()],
        })
        .await?)
}

/// Settles, retrying while the operators haven't yet recorded the receive.
async fn settle_when_received(
    server: &SdkInstance,
    payment: &str,
    url: &str,
) -> Result<SettleX402Response> {
    for _ in 0..30 {
        let response = settle(server, payment, url).await?;
        if response.error_reason.as_deref() != Some("exact_lnbtc_invoice_not_settled") {
            return Ok(response);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    settle(server, payment, url).await
}

fn pay_to(challenge: &CreateX402ChallengeResponse) -> Result<String> {
    let payment_required: serde_json::Value =
        serde_json::from_slice(&BASE64_STANDARD.decode(&challenge.payment_required)?)?;
    Ok(payment_required["accepts"][0]["payTo"]
        .as_str()
        .expect("payTo is a string")
        .to_string())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_pay_and_settle(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let server = env.create_wallet().await?;
    let mut client = env.create_wallet().await?;
    ensure_funded(&mut client, 20_000).await?;

    let challenge = challenge(&server, URL_A).await?;
    let payment = pay(&client, &challenge.payment_required, URL_A).await?;
    info!("Paid x402 challenge {}", challenge.payment_hash);

    // A proof for article A doesn't buy article B, and isn't used up trying.
    let other = settle_when_received(&server, &payment, URL_B).await?;
    assert!(!other.success);
    assert_eq!(
        other.error_reason.as_deref(),
        Some("invalid_exact_lnbtc_request_mismatch")
    );

    let settled = settle_when_received(&server, &payment, URL_A).await?;
    assert!(settled.success, "settle failed: {:?}", settled.error_reason);
    assert_eq!(
        settled.amount,
        Some(X402PaymentAmount::Lightning {
            amount_sats: PRICE_SATS
        })
    );
    assert_eq!(
        settled.payment_hash.as_deref(),
        Some(challenge.payment_hash.as_str())
    );

    let replay = settle(&server, &payment, URL_A).await?;
    assert!(!replay.success);
    assert_eq!(replay.error_reason.as_deref(), Some("duplicate_settlement"));
    Ok(())
}

/// Every Spark invoice is signed by the service provider's shared key, so
/// another wallet can issue a challenge for the same request that looks
/// identical. Its proof carries the other wallet's quote, so it must not buy
/// the request here.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_refuses_another_wallets_invoice(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    let server = env.create_wallet().await?;
    let other_server = env.create_wallet().await?;
    let mut client = env.create_wallet().await?;
    ensure_funded(&mut client, 20_000).await?;

    let ours = challenge(&server, URL_A).await?;
    let theirs = challenge(&other_server, URL_A).await?;
    assert_eq!(
        pay_to(&ours)?,
        pay_to(&theirs)?,
        "both are signed by the SSP"
    );

    let payment = pay(&client, &theirs.payment_required, URL_A).await?;
    let refused = settle(&server, &payment, URL_A).await?;
    assert!(!refused.success);
    assert_eq!(
        refused.error_reason.as_deref(),
        Some("invalid_exact_lnbtc_quote")
    );

    // The wallet that issued it does accept it.
    let settled = settle_when_received(&other_server, &payment, URL_A).await?;
    assert!(settled.success, "settle failed: {:?}", settled.error_reason);
    Ok(())
}
