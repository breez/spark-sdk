use std::time::Duration;

use platform_utils::{ProxyConfig, Socks5Connector};
use tonic::transport::ClientTlsConfig;

use super::retry_channel::RetryChannel;
use crate::{default_user_agent, operator::rpc::OperatorRpcError};

pub type Transport = RetryChannel<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    /// A lazily-connected channel to the operator, tunnelled through `proxy`
    /// when one is set.
    ///
    /// The proxy only replaces how the TCP connection is opened. Tonic still
    /// applies the endpoint's TLS config on top, with the SNI name taken from
    /// `url`, so certificate validation is unchanged either way.
    pub fn new(
        url: String,
        ca_cert: Option<Vec<u8>>,
        user_agent: Option<String>,
        proxy: Option<&ProxyConfig>,
    ) -> Result<Self, OperatorRpcError> {
        let endpoint = EndpointTemplate::new(url, ca_cert, user_agent).build()?;
        let channel = match proxy {
            Some(proxy) => endpoint.connect_with_connector_lazy(Socks5Connector::new(proxy)),
            None => endpoint.connect_lazy(),
        };
        Ok(Self {
            inner: RetryChannel::new(channel),
        })
    }

    pub fn into_inner(self) -> Transport {
        self.inner
    }
}

#[derive(Clone)]
pub struct EndpointTemplate {
    url: String,
    ca_cert: Option<Vec<u8>>,
    user_agent: Option<String>,
}

impl EndpointTemplate {
    pub fn new(url: String, ca_cert: Option<Vec<u8>>, user_agent: Option<String>) -> Self {
        Self {
            url,
            ca_cert,
            user_agent,
        }
    }

    pub fn build(&self) -> Result<tonic::transport::Endpoint, OperatorRpcError> {
        let client_tls_config = match &self.ca_cert {
            Some(ca_cert) => ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(ca_cert)),
            None => ClientTlsConfig::new().with_webpki_roots(),
        };
        Ok(tonic::transport::Endpoint::from_shared(self.url.clone())?
            .tls_config(client_tls_config)?
            .http2_keep_alive_interval(Duration::new(5, 0))
            .tcp_keepalive(Some(Duration::from_secs(5)))
            .keep_alive_timeout(Duration::from_secs(5))
            .keep_alive_while_idle(true)
            .timeout(Duration::from_secs(60))
            .user_agent(self.user_agent.clone().unwrap_or_else(default_user_agent))?)
    }
}

impl From<tonic::transport::Error> for OperatorRpcError {
    fn from(error: tonic::transport::Error) -> Self {
        OperatorRpcError::Transport(error.to_string())
    }
}
