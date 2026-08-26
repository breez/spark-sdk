use std::collections::HashMap;
use tokio::sync::RwLock;
use tonic::transport::Channel;
use tracing::debug;

use crate::operator::OperatorConfig;
use crate::operator::rpc::error::Result;
use crate::operator::rpc::transport::grpc_client::{EndpointTemplate, Transport};
use crate::operator::rpc::transport::retry_channel::RetryChannel;

use super::ConnectionManager;

/// Opens several connections per operator and balances requests across them.
///
/// This manager cannot be proxied. Balancing goes through
/// [`Channel::balance_list`], which builds its own connectors and has no
/// connector-aware variant in any tonic release, so there is nowhere to insert
/// a SOCKS5 dialer. The SDK rejects a config that asks for both; use
/// [`DefaultConnectionManager`](super::DefaultConnectionManager) with a proxy.
pub struct BalancedConnectionManager {
    connections_map: RwLock<HashMap<String, Transport>>,
    connections_per_operator: u32,
}

impl BalancedConnectionManager {
    pub fn new(connections_per_operator: u32) -> Self {
        Self {
            connections_map: RwLock::new(HashMap::new()),
            connections_per_operator: connections_per_operator.max(1),
        }
    }
}

#[macros::async_trait]
impl ConnectionManager for BalancedConnectionManager {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport> {
        let key = operator.address.to_string();
        if let Some(transport) = self.connections_map.read().await.get(&key) {
            return Ok(transport.clone());
        }

        let mut map = self.connections_map.write().await;
        if let Some(transport) = map.get(&key) {
            return Ok(transport.clone());
        }

        let template = EndpointTemplate::new(
            operator.address.to_string(),
            operator.ca_cert.clone(),
            operator.user_agent.clone(),
        );
        let endpoints = (0..self.connections_per_operator)
            .map(|_| template.build())
            .collect::<Result<Vec<_>>>()?;
        let transport = RetryChannel::new(Channel::balance_list(endpoints.into_iter()));

        map.insert(key, transport.clone());
        debug!(
            "Created {} balanced connections to operator: {}",
            self.connections_per_operator, operator.address
        );
        Ok(transport)
    }
}
