use std::collections::HashMap;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tracing::debug;

use crate::operator::Operator;

use super::error::{OperatorRpcError, Result};

pub struct ConnectionManager {
    connections_map: Mutex<HashMap<String, Channel>>,
}

impl ConnectionManager {
    pub fn new() -> ConnectionManager {
        let connections_map = HashMap::new();
        Self {
            connections_map: Mutex::new(connections_map),
        }
    }

    pub async fn get_channel(&self, operator: &Operator) -> Result<Channel> {
        let mut map = self.connections_map.lock().await;
        let operator_connection = map.get(&operator.address.to_string());
        match operator_connection {
            Some(operator_connection) => Ok(operator_connection.clone()),
            None => {
                let channel = Channel::from_shared(operator.address.to_string())
                    .map_err(|e| OperatorRpcError::InvalidUri(e.to_string()))?
                    .connect_lazy();

                map.insert(operator.address.to_string(), channel.clone());
                debug!("Created new connection to operator: {}", operator.address);
                Ok(channel)
            }
        }
    }
}
