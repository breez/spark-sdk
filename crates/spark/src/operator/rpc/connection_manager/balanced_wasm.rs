use crate::operator::OperatorConfig;
use crate::operator::rpc::error::Result;
use crate::operator::rpc::transport::grpc_client::Transport;

use super::{ConnectionManager, DefaultConnectionManager};

pub struct BalancedConnectionManager(DefaultConnectionManager);

impl BalancedConnectionManager {
    pub fn new(_connections_per_operator: u32) -> Self {
        Self(DefaultConnectionManager::new())
    }
}

#[macros::async_trait]
impl ConnectionManager for BalancedConnectionManager {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport> {
        self.0.get_transport(operator).await
    }
}
