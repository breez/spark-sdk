use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use platform_utils::{
    ContentType, DefaultHttpClient, HttpClient, add_basic_auth_header, add_content_type_header,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tracing::{debug, info};

/// Global semaphore to limit concurrent faucet requests.
/// This enables test parallelization while preventing faucet rate limiting.
/// The number of permits can be configured via FAUCET_CONCURRENCY env var (default: 2).
static FAUCET_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    let concurrency = std::env::var("FAUCET_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    info!(
        "Initialized faucet semaphore with {} concurrent permits",
        concurrency
    );
    Semaphore::new(concurrency)
});

/// Configuration for the regtest faucet
#[derive(Debug, Clone)]
pub struct FaucetConfig {
    /// GraphQL endpoint URL for the faucet
    /// Default: https://app.lightspark.com/graphql/spark/rc
    pub url: String,
    /// Optional username for basic authentication
    /// Can be set with FAUCET_USERNAME environment variable
    pub username: Option<String>,
    /// Optional password for basic authentication
    /// Can be set with FAUCET_PASSWORD environment variable
    pub password: Option<String>,
}

impl Default for FaucetConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("FAUCET_URL")
                .unwrap_or_else(|_| "https://api.lightspark.com/graphql/spark/rc".to_string()),
            username: std::env::var("FAUCET_USERNAME").ok(),
            password: std::env::var("FAUCET_PASSWORD").ok(),
        }
    }
}

/// Client for interacting with a regtest faucet
pub struct RegtestFaucet {
    config: FaucetConfig,
    http_client: DefaultHttpClient,
}

#[derive(Debug, Serialize)]
struct GraphQLRequest {
    #[serde(rename = "operationName")]
    operation_name: String,
    variables: FaucetVariables,
    query: String,
}

#[derive(Debug, Serialize)]
struct FaucetVariables {
    amount_sats: u64,
    address: String,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<ResponseData>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct ResponseData {
    request_regtest_funds: RequestRegtestFunds,
}

#[derive(Debug, Deserialize)]
struct RequestRegtestFunds {
    transaction_hash: String,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

impl RegtestFaucet {
    /// Create a new faucet client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(FaucetConfig::default())
    }

    /// Create a new faucet client with custom configuration
    pub fn with_config(config: FaucetConfig) -> Result<Self> {
        info!("Initialized faucet client with URL: {}", config.url);
        Ok(Self {
            config,
            http_client: DefaultHttpClient::default(),
        })
    }

    /// Fund an address with the specified amount
    ///
    /// This method acquires a permit from the global faucet semaphore before making
    /// the request, ensuring controlled concurrency when tests run in parallel.
    ///
    /// # Arguments
    /// * `address` - Bitcoin regtest address to fund
    /// * `amount_sats` - Amount in satoshis to send
    ///
    /// # Returns
    /// The transaction hash of the funding transaction
    pub async fn fund_address(&self, address: &str, amount_sats: u64) -> Result<String> {
        // Acquire semaphore permit to limit concurrent faucet requests
        let _permit = FAUCET_SEMAPHORE
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to acquire faucet semaphore: {}", e))?;

        info!(
            "Requesting funds from faucet: {} sats to address {}",
            amount_sats, address
        );

        let request_body = GraphQLRequest {
            operation_name: "RequestRegtestFunds".to_string(),
            variables: FaucetVariables {
                amount_sats,
                address: address.to_string(),
            },
            query: "mutation RequestRegtestFunds($address: String!, $amount_sats: Long!) { request_regtest_funds(input: {address: $address, amount_sats: $amount_sats}) { transaction_hash}}".to_string(),
        };

        debug!("Sending GraphQL request: {:?}", request_body);

        let body_json =
            serde_json::to_string(&request_body).context("Failed to serialize request body")?;

        let mut headers = HashMap::new();
        add_content_type_header(&mut headers, ContentType::Json);

        // Add basic authentication if username and password are configured
        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            add_basic_auth_header(&mut headers, username, password);
        }

        let response = self
            .http_client
            .post(self.config.url.clone(), Some(headers), Some(body_json))
            .await
            .context("Failed to send faucet request")?;

        if !response.is_success() {
            bail!(
                "Faucet request failed with status {}: {}",
                response.status,
                response.body
            );
        }

        let graphql_response: GraphQLResponse =
            response.json().context(response.body.to_string())?;

        // Check for GraphQL errors
        if let Some(errors) = graphql_response.errors {
            let error_messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            bail!("Faucet returned errors: {}", error_messages.join(", "));
        }

        // Extract transaction hash
        let txid = graphql_response
            .data
            .ok_or_else(|| anyhow::anyhow!("Faucet response missing data"))?
            .request_regtest_funds
            .transaction_hash;

        info!("Successfully funded address, transaction hash: {}", txid);
        Ok(txid)
    }
}

impl Default for RegtestFaucet {
    fn default() -> Self {
        Self::new().expect("Failed to create default faucet client")
    }
}
