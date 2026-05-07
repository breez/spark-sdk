use std::sync::Arc;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use spark_wallet::BalancedConnectionManager;
use spark_wallet::{ConnectionManager as InnerConnectionManager, DefaultConnectionManager};

/// A shareable manager for gRPC connections to the Spark operators.
///
/// Construct one via [`new_connection_manager`] (single connection per
/// operator, suitable for most uses) or [`new_balanced_connection_manager`]
/// (auto-pooled connections per operator, for high-throughput backends).
///
/// Pass the same `Arc` to multiple [`SdkBuilder`](crate::SdkBuilder)s via
/// [`SdkBuilder::with_connection_manager`](crate::SdkBuilder::with_connection_manager).
/// Connections close when the last reference to the connection manager is
/// dropped; calling [`BreezSdk::disconnect`](crate::BreezSdk::disconnect) does
/// not affect them.
///
/// All SDK instances sharing a `ConnectionManager` must be configured for the
/// same network and operator pool: the cache is keyed only by operator address,
/// so the TLS settings and user agent of the first SDK to connect are reused
/// for everyone afterwards.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct ConnectionManager {
    pub(crate) inner: ConnectionManagerInner,
}

pub(crate) enum ConnectionManagerInner {
    Default(Arc<DefaultConnectionManager>),
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    Balanced(Arc<BalancedConnectionManager>),
}

impl ConnectionManager {
    /// Returns a per-tenant handle on the underlying transport pool. Each
    /// `BreezSdk` instance gets its own handle; the handle owns the registration
    /// against pool-aware managers and releases it on drop.
    #[allow(
        clippy::unused_async,
        reason = "balanced arm awaits; default arm doesn't"
    )]
    pub(crate) async fn for_tenant(&self) -> Arc<dyn InnerConnectionManager> {
        match &self.inner {
            ConnectionManagerInner::Default(cm) => cm.clone(),
            #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
            ConnectionManagerInner::Balanced(cm) => cm.register_tenant().await,
        }
    }
}

/// Creates a connection manager.
///
/// - `None` opens one gRPC connection per operator and reuses it for every
///   SDK sharing this manager. HTTP/2 multiplexing handles concurrent calls
///   over that single connection. Suitable for most workloads.
/// - `Some(max_tenants_per_connection)` opens an additional connection per
///   operator for every `max_tenants_per_connection` SDKs sharing this
///   manager. Use for high-throughput backends that need multiple connections
///   per operator to mitigate stream caps, TCP head-of-line blocking, or
///   L7-LB stickiness. The pool only grows; it does not shrink when SDKs
///   disconnect, to avoid TCP slow-start churn on transient restarts.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[must_use]
pub fn new_connection_manager(max_tenants_per_connection: Option<u32>) -> Arc<ConnectionManager> {
    let inner = match max_tenants_per_connection {
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        Some(n) => ConnectionManagerInner::Balanced(Arc::new(BalancedConnectionManager::new(n))),
        // On WASM the parameter is accepted for API symmetry but ignored: the
        // browser and Node's undici own the underlying HTTP connection pool
        // and the SDK has no useful pooling knob to expose.
        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        Some(_) => ConnectionManagerInner::Default(Arc::new(DefaultConnectionManager::new())),
        None => ConnectionManagerInner::Default(Arc::new(DefaultConnectionManager::new())),
    };
    Arc::new(ConnectionManager { inner })
}
