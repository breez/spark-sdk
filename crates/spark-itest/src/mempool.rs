use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bitcoin::{Transaction, consensus::encode::deserialize_hex};
use platform_utils::{DefaultHttpClient, HttpClient, HttpResponse, add_basic_auth_header};
use tracing::info;

const MAX_RETRIES: usize = 5;
const BASE_BACKOFF: Duration = Duration::from_millis(256);
const RETRYABLE_STATUSES: [u16; 3] = [429, 500, 503];

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
    config: MempoolConfig,
    http_client: DefaultHttpClient,
}

impl MempoolClient {
    /// Create a new mempool client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(MempoolConfig::default())
    }

    /// Create a new mempool client with custom configuration
    pub fn with_config(config: MempoolConfig) -> Result<Self> {
        info!("Initialized mempool client with URL: {}", config.url);
        Ok(Self {
            config,
            http_client: DefaultHttpClient::new(None).expect("http client"),
        })
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

        let response = self.get_with_retry(&url).await?;

        if !response.is_success() {
            bail!(
                "Failed to fetch transaction {}: status {}, body: {}",
                txid,
                response.status,
                response.body
            );
        }

        let hex = &response.body;
        let tx: Transaction = deserialize_hex(hex)
            .context(format!("Failed to deserialize transaction hex: {}", hex))?;

        info!("Successfully fetched transaction: {}", txid);
        Ok(tx)
    }

    /// The shared regtest esplora reaps idle HTTP/2 flows, so the next request
    /// off a pooled connection can come back as a broken pipe with no response
    /// at all. Those, and the endpoint's own transient statuses, get another
    /// attempt before the caller sees a failure.
    async fn get_with_retry(&self, url: &str) -> Result<HttpResponse> {
        let mut delay = BASE_BACKOFF;
        let mut last_error;

        for attempt in 0..=MAX_RETRIES {
            let mut headers = HashMap::new();
            add_basic_auth_header(&mut headers, &self.config.username, &self.config.password);

            match self.http_client.get(url.to_string(), Some(headers)).await {
                Ok(response) if !RETRYABLE_STATUSES.contains(&response.status) => {
                    return Ok(response);
                }
                Ok(response) => {
                    last_error =
                        anyhow::anyhow!("status {}, body: {}", response.status, response.body);
                }
                Err(e) => last_error = anyhow::anyhow!("{e}"),
            }

            if attempt == MAX_RETRIES {
                return Err(last_error).context("Failed to fetch transaction");
            }
            tokio::time::sleep(delay).await;
            delay = delay.saturating_mul(2);
        }

        unreachable!("the loop returns on its final attempt")
    }
}

impl Default for MempoolClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default mempool client")
    }
}
