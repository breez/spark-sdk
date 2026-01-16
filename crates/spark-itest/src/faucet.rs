use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

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
    client: Client,
    config: FaucetConfig,
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
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        info!("Initialized faucet client with URL: {}", config.url);
        Ok(Self { client, config })
    }

    /// Fund an address with the specified amount
    ///
    /// # Arguments
    /// * `address` - Bitcoin regtest address to fund
    /// * `amount_sats` - Amount in satoshis to send
    ///
    /// # Returns
    /// The transaction hash of the funding transaction
    pub async fn fund_address(&self, address: &str, amount_sats: u64) -> Result<String> {
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

        let mut req = self.client.post(&self.config.url).json(&request_body);

        // Add basic authentication if username and password are configured
        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            req = req.basic_auth(username, Some(password));
        }
        req = req.header("Content-Type", "application/json");

        let response = req.send().await.context("Failed to send faucet request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Faucet request failed with status {}: {}", status, body);
        }

        let response_text = response.text().await?;
        let graphql_response: GraphQLResponse =
            serde_json::from_str(&response_text).context(response_text)?;

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
