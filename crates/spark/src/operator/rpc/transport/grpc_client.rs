use std::time::Duration;

use tonic::transport::ClientTlsConfig;

use super::retry_channel::RetryChannel;
use crate::operator::rpc::OperatorRpcError;
pub type Transport = RetryChannel<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    pub fn new(url: String) -> Result<Self, OperatorRpcError> {
        Ok(Self {
            inner: RetryChannel::new(Self::create_endpoint(&url)?.connect_lazy()),
        })
    }

    pub fn into_inner(self) -> Transport {
        self.inner
    }

    fn create_endpoint(server_url: &str) -> Result<tonic::transport::Endpoint, OperatorRpcError> {
        Ok(
            tonic::transport::Endpoint::from_shared(server_url.to_string())?
                .tls_config(ClientTlsConfig::new().with_enabled_roots())?
                .http2_keep_alive_interval(Duration::new(5, 0))
                .tcp_keepalive(Some(Duration::from_secs(5)))
                .keep_alive_timeout(Duration::from_secs(5))
                .keep_alive_while_idle(true),
        )
    }
}

impl From<tonic::transport::Error> for OperatorRpcError {
    fn from(error: tonic::transport::Error) -> Self {
        OperatorRpcError::Transport(error.to_string())
    }
}
