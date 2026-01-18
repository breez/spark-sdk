use anyhow::{Context, Result, bail};
use bitcoin::{Transaction, consensus::encode::deserialize_hex};
use reqwest::Client;
use std::time::Duration;
use tracing::info;

/// Configuration for the mempool/esplora API client
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Base URL for the mempool/esplora API
    /// Default: https://regtest-mempool.us-west-2.sparkinfra.net/api
    pub url: String,
    /// Username for basic authentication
    pub username: String,
    /// Password for basic authentication
    pub password: String,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("MEMPOOL_URL").unwrap_or_else(|_| {
                "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string()
            }),
            username: std::env::var("MEMPOOL_USERNAME").unwrap_or_else(|_| "spark-sdk".to_string()),
            password: std::env::var("MEMPOOL_PASSWORD")
                .unwrap_or_else(|_| "mCMk1JqlBNtetUNy".to_string()),
        }
    }
}

/// Client for fetching transactions from a mempool/esplora API
pub struct MempoolClient {
    client: Client,
    config: MempoolConfig,
}

impl MempoolClient {
    /// Create a new mempool client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(MempoolConfig::default())
    }

    /// Create a new mempool client with custom configuration
    pub fn with_config(config: MempoolConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        info!("Initialized mempool client with URL: {}", config.url);
        Ok(Self { client, config })
    }

    /// Fetch a transaction by its txid
    ///
    /// # Arguments
    /// * `txid` - The transaction ID to fetch
    ///
    /// # Returns
    /// The deserialized Bitcoin transaction
    pub async fn get_transaction(&self, txid: &str) -> Result<Transaction> {
        let url = format!("{}/tx/{}/hex", self.config.url, txid);
        info!("Fetching transaction from: {}", url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .send()
            .await
            .context("Failed to fetch transaction")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(
                "Failed to fetch transaction {}: status {}, body: {}",
                txid,
                status,
                body
            );
        }

        let hex = response.text().await?;
        let tx: Transaction = deserialize_hex(&hex)
            .context(format!("Failed to deserialize transaction hex: {}", hex))?;

        info!("Successfully fetched transaction: {}", txid);
        Ok(tx)
    }
}

impl Default for MempoolClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default mempool client")
    }
}
