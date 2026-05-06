use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::debug;

use crate::operator::{
    OperatorConfig,
    rpc::transport::grpc_client::{GrpcClient, Transport},
};

use super::error::Result;

#[macros::async_trait]
pub trait ConnectionManager: Send + Sync {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport>;
}

pub struct DefaultConnectionManager {
    connections_map: RwLock<HashMap<String, Transport>>,
}

impl Default for DefaultConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultConnectionManager {
    pub fn new() -> Self {
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        {
            // Install rustls ring crypto provider for native targets only
            if rustls::crypto::ring::default_provider()
                .install_default()
                .is_err()
            {
                tracing::warn!("Failed to install rustls crypto provider, ignoring error");
            }
        }
        Self {
            connections_map: RwLock::new(HashMap::new()),
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
        )?
        .into_inner();

        map.insert(key, transport.clone());
        debug!("Created new connection to operator: {}", operator.address);
        Ok(transport)
    }
}
