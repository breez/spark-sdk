use crate::ssp::graphql::GraphQLClientConfig;

mod error;
mod graphql;
mod service_provider;

use bitcoin::secp256k1::PublicKey;
pub use error::ServiceProviderError;
pub use graphql::models::*;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
pub use service_provider::ServiceProvider;

/// Config for creating a ServiceProvider
#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceProviderConfig {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,
    /// Identity public key of the service provider
    #[serde_as(as = "DisplayFromStr")]
    pub identity_public_key: PublicKey,
    pub user_agent: Option<String>,
    /// Retry policy for transient 5xx responses from the SSP.
    #[serde(default)]
    pub retry_config: RetryConfig,
}

/// Retry policy for transient 5xx responses from the SSP GraphQL endpoint.
///
/// The first retry waits `base_delay_ms` (plus up to 50% jitter), and each
/// subsequent retry doubles the base delay, capped at `max_delay_ms`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts after the initial request fails with a 5xx.
    pub max_retries: u32,
    /// Initial backoff delay in milliseconds; doubled on each subsequent attempt.
    pub base_delay_ms: u64,
    /// Upper bound on the exponential backoff delay in milliseconds (excluding jitter).
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay_ms: 50,
            max_delay_ms: 500,
        }
    }
}

impl From<ServiceProviderConfig> for GraphQLClientConfig {
    fn from(opts: ServiceProviderConfig) -> Self {
        Self {
            base_url: opts.base_url,
            schema_endpoint: opts.schema_endpoint,
            ssp_identity_public_key: opts.identity_public_key,
            user_agent: opts.user_agent,
            retry_config: opts.retry_config,
        }
    }
}

// TODO: handle the case where the currency is not sats
impl CurrencyAmount {
    pub fn as_sats(&self) -> Result<u64, ServiceProviderError> {
        match self.original_unit {
            CurrencyUnit::Millisatoshi => Ok(self.original_value.div_ceil(1000)),
            CurrencyUnit::Satoshi => Ok(self.original_value),
            _ => Err(ServiceProviderError::ParseError(
                "Unsupported currency unit".to_string(),
            )),
        }
    }
}
