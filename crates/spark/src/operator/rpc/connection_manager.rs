use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::debug;

use crate::operator::{
    OperatorConfig,
    rpc::transport::grpc_client::{GrpcClient, Transport},
};

use super::error::Result;

pub struct ConnectionManager {
    connections_map: Mutex<HashMap<String, Transport>>,
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionManager {
    pub fn new() -> ConnectionManager {
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        {
            // Install rustls ring crypto provider for native targets only
            if rustls::crypto::ring::default_provider()
                .install_default()
                .is_err()
            {
                tracing::error!("Failed to install rustls crypto provider, ignoring error");
            }
        }
        let connections_map = HashMap::new();
        Self {
            connections_map: Mutex::new(connections_map),
        }
    }

    pub async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport> {
        let mut map = self.connections_map.lock().await;
        let operator_connection = map.get(&operator.address.to_string());
        match operator_connection {
            Some(operator_connection) => Ok(operator_connection.clone()),
            None => {
                let transport = GrpcClient::new(operator.address.to_string())?.into_inner();

                map.insert(operator.address.to_string(), transport.clone());
                debug!("Created new connection to operator: {}", operator.address);
                Ok(transport)
            }
        }
    }
}
