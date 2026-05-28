//! Flutter wrapper around [`breez_sdk_spark::SdkContext`].
//!
//! Construct once via [`new_shared_sdk_context`] and pass the same handle to
//! every [`SdkBuilder`](crate::sdk_builder::SdkBuilder) via
//! [`with_shared_context`](crate::sdk_builder::SdkBuilder::with_shared_context).
//! All SDKs
//! sharing the context reuse one HTTP client (SSP / chain / LNURL / JWT) and
//! one set of gRPC channels to the Spark operators and the Breez backend.

use std::sync::Arc;

use breez_sdk_spark::SdkError;

use crate::models::SdkContextConfig;

pub struct SdkContext {
    pub(crate) inner: Arc<breez_sdk_spark::SdkContext>,
}

/// Process-shared SDK resources for Flutter integrations.
pub async fn new_shared_sdk_context(config: SdkContextConfig) -> Result<SdkContext, SdkError> {
    let inner = breez_sdk_spark::new_shared_sdk_context(breez_sdk_spark::SdkContextConfig {
        network: config.network,
        api_key: config.api_key,
        connections_per_operator: config.connections_per_operator,
        storage: None,
    })
    .await?;
    Ok(SdkContext { inner })
}
