use crate::ssp::graphql::GraphQLClientConfig;

mod error;
mod graphql;
mod service_provider;

pub use graphql::types::*;
pub use service_provider::ServiceProvider;

/// Config for creating a ServiceProvider
#[derive(Debug, Clone)]
pub struct ServiceProviderConfig {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,
}

impl From<ServiceProviderConfig> for GraphQLClientConfig {
    fn from(opts: ServiceProviderConfig) -> Self {
        Self {
            base_url: opts.base_url,
            schema_endpoint: opts.schema_endpoint,
        }
    }
}
