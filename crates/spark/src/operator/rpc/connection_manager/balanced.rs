use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, mpsc};
use tonic::transport::{Channel, Endpoint};
use tower::discover::Change;
use tracing::{debug, warn};

use crate::operator::OperatorConfig;
use crate::operator::rpc::error::Result;
use crate::operator::rpc::transport::grpc_client::{EndpointTemplate, Transport};
use crate::operator::rpc::transport::retry_channel::RetryChannel;

use super::ConnectionManager;

const POOL_CHANGE_BUFFER: usize = 16;

struct OperatorConnectionState {
    transport: Transport,
    pool_change_tx: mpsc::Sender<Change<u64, Endpoint>>,
    next_endpoint_key: u64,
    current_pool_size: usize,
    endpoint_template: EndpointTemplate,
}

/// Pooling connection manager whose underlying connection count grows
/// with the number of registered tenants.
///
/// Register each tenant via [`register_tenant`](Self::register_tenant) to
/// obtain a per-tenant [`ConnectionManager`] handle. Drop the handle to
/// deregister.
///
/// Pool size formula: `ceil(active_tenants / max_tenants_per_connection)`.
/// The pool grows only; it does not shrink when tenants drop, to avoid
/// TCP slow-start churn during transient reconnects.
pub struct BalancedConnectionManager {
    inner: Arc<BalancedInner>,
}

struct BalancedInner {
    max_tenants_per_connection: u32,
    active_tenants: AtomicUsize,
    operators: Mutex<HashMap<String, OperatorConnectionState>>,
}

impl BalancedConnectionManager {
    /// Creates a new balanced manager.
    ///
    /// `max_tenants_per_connection` controls how many tenants share each
    /// underlying connection before another is opened. Must be at least 1.
    pub fn new(max_tenants_per_connection: u32) -> Self {
        if rustls::crypto::ring::default_provider()
            .install_default()
            .is_err()
        {
            tracing::debug!("Failed to install rustls crypto provider, ignoring error");
        }
        Self {
            inner: Arc::new(BalancedInner {
                max_tenants_per_connection: max_tenants_per_connection.max(1),
                active_tenants: AtomicUsize::new(0),
                operators: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Registers a tenant and returns a per-tenant [`ConnectionManager`]
    /// handle. Drop the handle to deregister.
    pub async fn register_tenant(&self) -> Arc<dyn ConnectionManager> {
        self.inner.active_tenants.fetch_add(1, Ordering::SeqCst);
        self.inner.grow_existing_pools().await;
        Arc::new(BalancedTenantHandle {
            inner: self.inner.clone(),
        })
    }
}

struct BalancedTenantHandle {
    inner: Arc<BalancedInner>,
}

#[macros::async_trait]
impl ConnectionManager for BalancedTenantHandle {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport> {
        self.inner.get_transport(operator).await
    }
}

impl Drop for BalancedTenantHandle {
    fn drop(&mut self) {
        self.inner.active_tenants.fetch_sub(1, Ordering::SeqCst);
    }
}

impl BalancedInner {
    fn desired_pool_size(&self) -> usize {
        self.active_tenants
            .load(Ordering::SeqCst)
            .div_ceil(self.max_tenants_per_connection as usize)
            .max(1)
    }

    async fn grow_existing_pools(&self) {
        let target = self.desired_pool_size();
        let mut operators = self.operators.lock().await;
        for state in operators.values_mut() {
            grow_pool_to(state, target);
        }
    }

    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport> {
        let key = operator.address.to_string();
        let mut operators = self.operators.lock().await;
        if let Some(state) = operators.get(&key) {
            return Ok(state.transport.clone());
        }

        let endpoint_template = EndpointTemplate::new(
            operator.address.to_string(),
            operator.ca_cert.clone(),
            operator.user_agent.clone(),
        );

        let (channel, tx) = Channel::balance_channel::<u64>(POOL_CHANGE_BUFFER);
        let mut state = OperatorConnectionState {
            transport: RetryChannel::new(channel),
            pool_change_tx: tx,
            next_endpoint_key: 0,
            current_pool_size: 0,
            endpoint_template,
        };
        grow_pool_to(&mut state, self.desired_pool_size());

        let transport = state.transport.clone();
        operators.insert(key, state);
        debug!("Created new connection to operator: {}", operator.address);
        Ok(transport)
    }
}

fn grow_pool_to(state: &mut OperatorConnectionState, target: usize) {
    while state.current_pool_size < target {
        let endpoint = match state.endpoint_template.build() {
            Ok(ep) => ep,
            Err(e) => {
                warn!("Failed to build endpoint while growing pool: {e}");
                return;
            }
        };
        let key = state.next_endpoint_key;
        state.next_endpoint_key = state.next_endpoint_key.wrapping_add(1);
        if state
            .pool_change_tx
            .try_send(Change::Insert(key, endpoint))
            .is_err()
        {
            warn!("Failed to grow connection pool: change channel full or closed");
            return;
        }
        state.current_pool_size += 1;
    }
}
