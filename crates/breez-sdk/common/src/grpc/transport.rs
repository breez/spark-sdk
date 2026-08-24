use anyhow::Result;
use platform_utils::{ProxyConfig, Socks5Connector};
use std::time::Duration;
use tonic::transport::ClientTlsConfig;

pub type Transport = tonic::transport::Channel;

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    /// A lazily-connected channel to `url`, tunnelled through `proxy` when one
    /// is set.
    ///
    /// The proxy only replaces how the TCP connection is opened. Tonic still
    /// applies the endpoint's TLS config on top, with the SNI name taken from
    /// `url`, so certificate validation is unchanged either way.
    pub fn new(url: &str, user_agent: &str, proxy: Option<&ProxyConfig>) -> Result<Self> {
        let endpoint = Self::create_endpoint(url, user_agent)?;
        let inner = match proxy {
            Some(proxy) => endpoint.connect_with_connector_lazy(Socks5Connector::new(proxy)),
            None => endpoint.connect_lazy(),
        };
        Ok(Self { inner })
    }

    pub fn into_inner(self) -> Transport {
        self.inner
    }

    fn create_endpoint(server_url: &str, user_agent: &str) -> Result<tonic::transport::Endpoint> {
        Ok(
            tonic::transport::Endpoint::from_shared(server_url.to_string())?
                .tls_config(ClientTlsConfig::new().with_webpki_roots())?
                .http2_keep_alive_interval(Duration::new(5, 0))
                .tcp_keepalive(Some(Duration::from_secs(5)))
                .keep_alive_timeout(Duration::from_secs(5))
                .keep_alive_while_idle(true)
                .user_agent(user_agent)?,
        )
    }
}
