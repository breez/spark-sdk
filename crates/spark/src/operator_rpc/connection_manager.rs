use std::sync::Mutex;

use spark_protos::spark::spark_service_client::SparkServiceClient;

use log::debug;
use std::collections::HashMap;
use tonic::transport::Channel;

use super::error::{OperatorRpcError, Result};

pub struct ConnectionManager {
    identity_pubkey: Vec<u8>,
    connections_map: Mutex<HashMap<String, OperatorConnection>>,
}

struct OperatorConnection {
    channel: Channel,
    auth_token: Option<String>,
    expiration_time: u64,
}

impl ConnectionManager {
    pub fn new(identity_pubkey: Vec<u8>) -> Result<ConnectionManager> {
        let connections_map = HashMap::new();
        Ok(Self {
            identity_pubkey,
            connections_map: Mutex::new(connections_map),
        })
    }

    //TODO: We should return here an authenticated client preferably with some middleware.
    pub fn get_spark_service_client(&self, url: &str) -> Result<SparkServiceClient<Channel>> {
        let mut map = self.connections_map.lock().unwrap();
        let operator_connection = map.get(url);
        match operator_connection {
            Some(operator_connection) => {
                let channel = operator_connection.channel.clone();
                Ok(SparkServiceClient::new(channel.clone()))
            }
            None => {
                let channel = Channel::from_shared(url.to_string())
                    .map_err(|e| OperatorRpcError::InvalidUri(e.to_string()))?
                    .connect_lazy();

                map.insert(
                    url.to_string(),
                    OperatorConnection {
                        channel: channel.clone(),
                        auth_token: None,
                        expiration_time: 0,
                    },
                );
                debug!("Created new connection to operator: {}", url);
                Ok(SparkServiceClient::new(channel))
            }
        }
    }
}
