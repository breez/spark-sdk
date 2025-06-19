use std::collections::HashMap;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tracing::debug;

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

    pub async fn get_channel(&self, url: &str) -> Result<Channel> {
        let mut map = self.connections_map.lock().await;
        let operator_connection = map.get(url);
        match operator_connection {
            Some(operator_connection) => Ok(operator_connection.clone()),
            None => {
                let channel = Channel::from_shared(url.to_string())
                    .map_err(|e| OperatorRpcError::InvalidUri(e.to_string()))?
                    .connect_lazy();

                map.insert(url.to_string(), channel.clone());
                debug!("Created new connection to operator: {}", url);
                Ok(channel)
            }
        }
    }
}
