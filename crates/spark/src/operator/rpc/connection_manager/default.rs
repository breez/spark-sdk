use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::debug;

use crate::operator::OperatorConfig;
use crate::operator::rpc::error::Result;
use crate::operator::rpc::transport::grpc_client::{GrpcClient, Transport};

use super::ConnectionManager;

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
