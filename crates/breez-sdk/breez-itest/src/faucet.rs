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
    /// Maximum time to wait for funding confirmation (in seconds)
    pub confirmation_timeout_secs: u64,
}

impl Default for FaucetConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("FAUCET_URL")
                .unwrap_or_else(|_| "https://api.lightspark.com/graphql/spark/rc".to_string()),
            username: std::env::var("FAUCET_USERNAME").ok(),
            password: std::env::var("FAUCET_PASSWORD").ok(),
            confirmation_timeout_secs: 120,
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

    /// Fund an address and wait for the transaction to appear
    ///
    /// This is a convenience method that funds and then waits a moment for propagation
    pub async fn fund_and_wait(&self, address: &str, amount_sats: u64) -> Result<String> {
        let txid = self.fund_address(address, amount_sats).await?;

        info!(
            "Funded address {}, waiting for propagation... (txid: {})",
            address, txid
        );

        // In regtest, transactions should appear almost immediately
        // Just give it a moment to propagate
        tokio::time::sleep(Duration::from_secs(2)).await;

        Ok(txid)
    }
}

impl Default for RegtestFaucet {
    fn default() -> Self {
        Self::new().expect("Failed to create default faucet client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = FaucetConfig::default();
        assert!(!config.url.is_empty());
        assert!(config.url.contains("graphql"));
    }

    #[test]
    fn test_config_from_env() {
        unsafe {
            std::env::set_var("FAUCET_URL", "https://test.example.com/graphql");
            std::env::set_var("FAUCET_USERNAME", "test_user");
            std::env::set_var("FAUCET_PASSWORD", "test_pass");
        }

        let config = FaucetConfig::default();
        assert_eq!(config.url, "https://test.example.com/graphql");
        assert_eq!(config.username, Some("test_user".to_string()));
        assert_eq!(config.password, Some("test_pass".to_string()));

        unsafe {
            std::env::remove_var("FAUCET_URL");
            std::env::remove_var("FAUCET_USERNAME");
            std::env::remove_var("FAUCET_PASSWORD");
        }
    }

    #[test]
    fn test_faucet_client_creation() {
        let faucet = RegtestFaucet::new();
        assert!(faucet.is_ok());
    }
}
