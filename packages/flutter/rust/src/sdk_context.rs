//! Flutter wrapper around [`breez_sdk_spark::SdkContext`].
//!
//! Construct once via [`new_sdk_context`] and pass the same handle to every
//! [`SdkBuilder`](crate::sdk_builder::SdkBuilder) via
//! [`with_context`](crate::sdk_builder::SdkBuilder::with_context). All SDKs
//! sharing the context reuse one HTTP client (SSP / chain / LNURL / JWT) and
//! one set of gRPC channels to the Spark operators and the Breez backend.

use std::sync::Arc;

use flutter_rust_bridge::frb;

use breez_sdk_spark::{SdkContextConfig, SdkError};

pub struct SdkContext {
    pub(crate) inner: Arc<breez_sdk_spark::SdkContext>,
}

/// Process-shared SDK resources for Flutter integrations.
///
/// `connections_per_operator` controls per-operator gRPC connection pooling:
/// `None` (or `Some(1)`) keeps a single multiplexed connection per operator
/// (the right choice for almost every deployment); `Some(n)` opens `n`
/// connections per operator and balances requests across them.
#[frb(sync)]
pub fn new_sdk_context(connections_per_operator: Option<u32>) -> Result<SdkContext, SdkError> {
    let inner = breez_sdk_spark::new_sdk_context(SdkContextConfig {
        connections_per_operator,
    })?;
    Ok(SdkContext { inner })
}
