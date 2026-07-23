//! Minimal client for the Lightspark regtest faucet, mirroring
//! `breez-itest/src/faucet.rs` (not a dev-dependency: that crate pulls
//! testcontainers and database drivers into the plain workspace test job).
//! Concurrency is bounded by the runner's `--test-threads` instead of a
//! semaphore.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use platform_utils::{
    ContentType, DefaultHttpClient, HttpClient, add_basic_auth_header, add_content_type_header,
};
use serde::{Deserialize, Serialize};

const DEFAULT_URL: &str = "https://api.lightspark.com/graphql/spark/rc";
const MAX_RETRIES: u32 = 3;

#[derive(Serialize)]
struct GraphQlRequest {
    #[serde(rename = "operationName")]
    operation_name: String,
    variables: FaucetVariables,
    query: String,
}

#[derive(Serialize)]
struct FaucetVariables {
    amount_sats: u64,
    address: String,
}

#[derive(Deserialize)]
struct GraphQlResponse {
    data: Option<ResponseData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct ResponseData {
    request_regtest_funds: RequestRegtestFunds,
}

#[derive(Deserialize)]
struct RequestRegtestFunds {
    transaction_hash: String,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

/// Fund a regtest bitcoin address, returning the funding txid. Retries with
/// exponential backoff. Reads `FAUCET_URL`, `FAUCET_USERNAME`, and
/// `FAUCET_PASSWORD` from the environment.
pub async fn fund_address(address: &str, amount_sats: u64) -> Result<String> {
    let mut last_error = None;
    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
        }
        match try_fund_address(address, amount_sats).await {
            Ok(txid) => return Ok(txid),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.expect("at least one attempt ran"))
}

async fn try_fund_address(address: &str, amount_sats: u64) -> Result<String> {
    let url = std::env::var("FAUCET_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let request = GraphQlRequest {
        operation_name: "RequestRegtestFunds".to_string(),
        variables: FaucetVariables {
            amount_sats,
            address: address.to_string(),
        },
        query: "mutation RequestRegtestFunds($address: String!, $amount_sats: Long!) { \
                request_regtest_funds(input: {address: $address, amount_sats: $amount_sats}) \
                { transaction_hash}}"
            .to_string(),
    };
    let body = serde_json::to_string(&request).context("failed to serialize faucet request")?;

    let mut headers = HashMap::new();
    add_content_type_header(&mut headers, ContentType::Json);
    if let (Ok(username), Ok(password)) = (
        std::env::var("FAUCET_USERNAME"),
        std::env::var("FAUCET_PASSWORD"),
    ) {
        add_basic_auth_header(&mut headers, &username, &password);
    }

    let response = DefaultHttpClient::default()
        .post(url, Some(headers), Some(body))
        .await
        .context("faucet request failed")?;
    if !response.is_success() {
        bail!(
            "faucet request failed with status {}: {}",
            response.status,
            response.body
        );
    }

    let parsed: GraphQlResponse = response
        .json()
        .with_context(|| format!("unparseable faucet response: {}", response.body))?;
    if let Some(errors) = parsed.errors {
        let messages: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        bail!("faucet returned errors: {}", messages.join(", "));
    }
    let data = parsed.data.context("faucet response has no data")?;
    Ok(data.request_regtest_funds.transaction_hash)
}
