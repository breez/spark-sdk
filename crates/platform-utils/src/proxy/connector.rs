//! A `tower` connector that dials through a SOCKS5 proxy, for gRPC channels.
//!
//! `reqwest` handles its own proxying, so this exists for the tonic side,
//! which only takes a connector. TLS is layered on top by tonic itself, so the
//! stream handed back here is the plain tunnelled TCP connection.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use http::Uri;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;
use tower_service::Service;

use super::ProxyConfig;

#[derive(Debug, thiserror::Error)]
pub enum ProxyConnectError {
    #[error("proxy target URI has no host")]
    MissingHost,
    #[error("proxy target URI has no port and its scheme has no default")]
    MissingPort,
    #[error("SOCKS5 connection to {target} via {proxy} failed: {source}")]
    Connect {
        proxy: String,
        target: String,
        #[source]
        source: tokio_socks::Error,
    },
}

/// Dials every target through the configured SOCKS5 proxy.
///
/// The target host is sent to the proxy as a name, so the proxy performs the
/// lookup. Only the proxy's own address is resolved locally.
#[derive(Debug, Clone)]
pub struct Socks5Connector {
    proxy: Arc<ProxyConfig>,
}

impl Socks5Connector {
    #[must_use]
    pub fn new(proxy: &ProxyConfig) -> Self {
        Self {
            proxy: Arc::new(proxy.clone()),
        }
    }
}

/// Port from the URI, falling back to the scheme default. gRPC endpoints are
/// `http`/`https`, the only two schemes tonic hands a connector.
fn target_port(uri: &Uri) -> Option<u16> {
    uri.port_u16().or_else(|| match uri.scheme_str() {
        Some("https") => Some(443),
        Some("http") => Some(80),
        _ => None,
    })
}

impl Service<Uri> for Socks5Connector {
    type Response = TokioIo<TcpStream>;
    type Error = ProxyConnectError;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let proxy = Arc::clone(&self.proxy);
        Box::pin(async move {
            let host = uri
                .host()
                .ok_or(ProxyConnectError::MissingHost)?
                .to_string();
            let port = target_port(&uri).ok_or(ProxyConnectError::MissingPort)?;
            let proxy_addr = proxy.address();

            // `(String, u16)` becomes a `TargetAddr::Domain`, which is what
            // makes the proxy resolve the name instead of this process.
            let target = (host.clone(), port);
            let stream = match proxy.credentials() {
                Some((user, pass)) => {
                    Socks5Stream::connect_with_password(proxy_addr.as_str(), target, user, pass)
                        .await
                }
                None => Socks5Stream::connect(proxy_addr.as_str(), target).await,
            }
            .map_err(|source| ProxyConnectError::Connect {
                proxy: proxy_addr,
                target: format!("{host}:{port}"),
                source,
            })?;

            // Tonic only sets `TCP_NODELAY` on the `HttpConnector` it builds
            // itself, which a custom connector replaces, so a proxied channel
            // would otherwise run with Nagle on and pay up to ~40ms per small
            // gRPC write. Best-effort: matches tonic's own default.
            let stream = stream.into_inner();
            let _ = stream.set_nodelay(true);
            Ok(TokioIo::new(stream))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_falls_back_to_scheme_default() {
        assert_eq!(
            target_port(&Uri::from_static("https://example.com")),
            Some(443)
        );
        assert_eq!(
            target_port(&Uri::from_static("http://example.com")),
            Some(80)
        );
        assert_eq!(
            target_port(&Uri::from_static("https://example.com:8443")),
            Some(8443)
        );
    }

    #[tokio::test]
    async fn connect_failure_names_proxy_and_target() {
        // Port 1 has nothing listening, so the dial fails before any SOCKS
        // handshake and the error should still identify both endpoints.
        let proxy = ProxyConfig::new("127.0.0.1", 1);
        let mut connector = Socks5Connector::new(&proxy);
        let err = connector
            .call(Uri::from_static("https://example.com"))
            .await
            .expect_err("nothing is listening on 127.0.0.1:1");
        let msg = err.to_string();
        assert!(msg.contains("127.0.0.1:1"), "missing proxy: {msg}");
        assert!(msg.contains("example.com:443"), "missing target: {msg}");
    }
}
