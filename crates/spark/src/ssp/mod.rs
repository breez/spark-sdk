use crate::ssp::graphql::GraphQLClientOptions;

mod error;
mod graphql;
mod service_provider;

pub(crate) use graphql::types::*;
pub(crate) use service_provider::ServiceProvider;

/// Options for creating a ServiceProvider
#[derive(Debug, Clone)]
pub(crate) struct ServiceProviderOptions {
    /// Base URL for the GraphQL API
    pub base_url: String,
    /// Schema endpoint path (defaults to "graphql/spark/2025-03-19")
    pub schema_endpoint: Option<String>,
    /// Identity public key for authentication
    pub identity_public_key: String,
}

impl From<ServiceProviderOptions> for GraphQLClientOptions {
    fn from(opts: ServiceProviderOptions) -> Self {
        Self {
            base_url: opts.base_url,
            schema_endpoint: opts.schema_endpoint,
            identity_public_key: opts.identity_public_key,
        }
    }
}
