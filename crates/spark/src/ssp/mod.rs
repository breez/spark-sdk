use crate::ssp::graphql::GraphQLClientConfig;

mod error;
mod graphql;
mod service_provider;

use bitcoin::secp256k1::PublicKey;
pub use error::ServiceProviderError;
pub use graphql::types::*;
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
}

impl From<ServiceProviderConfig> for GraphQLClientConfig {
    fn from(opts: ServiceProviderConfig) -> Self {
        Self {
            base_url: opts.base_url,
            schema_endpoint: opts.schema_endpoint,
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
