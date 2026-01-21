use std::time::Duration;

use tonic::transport::ClientTlsConfig;

use super::retry_channel::RetryChannel;
use crate::{default_user_agent, operator::rpc::OperatorRpcError};
pub type Transport = RetryChannel<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    pub fn new(
        url: String,
        ca_cert: Option<Vec<u8>>,
        user_agent: Option<String>,
    ) -> Result<Self, OperatorRpcError> {
        Ok(Self {
            inner: RetryChannel::new(
                Self::create_endpoint(&url, ca_cert, user_agent)?.connect_lazy(),
            ),
        })
    }

    pub fn into_inner(self) -> Transport {
        self.inner
    }

    fn create_endpoint(
        server_url: &str,
        ca_cert: Option<Vec<u8>>,
        user_agent: Option<String>,
    ) -> Result<tonic::transport::Endpoint, OperatorRpcError> {
        let client_tls_config = match ca_cert {
            Some(ca_cert) => ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(ca_cert)),
            None => ClientTlsConfig::new().with_webpki_roots(),
        };
        Ok(
            tonic::transport::Endpoint::from_shared(server_url.to_string())?
                .tls_config(client_tls_config)?
                .http2_keep_alive_interval(Duration::new(5, 0))
                .tcp_keepalive(Some(Duration::from_secs(5)))
                .keep_alive_timeout(Duration::from_secs(5))
                .keep_alive_while_idle(true)
                .timeout(Duration::from_secs(30))
                .user_agent(user_agent.unwrap_or_else(default_user_agent))?,
        )
    }
}

impl From<tonic::transport::Error> for OperatorRpcError {
    fn from(error: tonic::transport::Error) -> Self {
        OperatorRpcError::Transport(error.to_string())
    }
}
