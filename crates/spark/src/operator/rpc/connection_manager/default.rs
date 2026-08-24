use std::collections::HashMap;

use platform_utils::ProxyConfig;
use tokio::sync::RwLock;
use tracing::debug;

use crate::operator::OperatorConfig;
use crate::operator::rpc::error::Result;
use crate::operator::rpc::transport::grpc_client::{GrpcClient, Transport};

use super::ConnectionManager;

pub struct DefaultConnectionManager {
    connections_map: RwLock<HashMap<String, Transport>>,
    proxy: Option<ProxyConfig>,
}

impl Default for DefaultConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultConnectionManager {
    pub fn new() -> Self {
        Self::with_proxy(None)
    }

    /// One connection per operator, tunnelled through `proxy` when set.
    #[must_use]
    pub fn with_proxy(proxy: Option<ProxyConfig>) -> Self {
        Self {
            connections_map: RwLock::new(HashMap::new()),
            proxy,
        }
    }
}

#[macros::async_trait]
impl ConnectionManager for DefaultConnectionManager {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport> {
        let key = operator.address.to_string();
        if let Some(transport) = self.connections_map.read().await.get(&key) {
            return Ok(transport.clone());
        }

        let mut map = self.connections_map.write().await;
        if let Some(transport) = map.get(&key) {
            return Ok(transport.clone());
        }

        let transport = GrpcClient::new(
            operator.address.to_string(),
            operator.ca_cert.clone(),
            operator.user_agent.clone(),
            self.proxy.as_ref(),
        )?
        .into_inner();

        map.insert(key, transport.clone());
        debug!("Created new connection to operator: {}", operator.address);
        Ok(transport)
    }
}
