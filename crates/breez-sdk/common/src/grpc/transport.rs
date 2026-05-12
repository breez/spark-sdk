use anyhow::Result;
use std::sync::Once;
use std::time::Duration;
use tonic::transport::ClientTlsConfig;

pub type Transport = tonic::transport::Channel;

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    pub fn new(url: &str, user_agent: &str) -> Result<Self> {
        // tonic's `ClientTlsConfig::with_webpki_roots` builds a `rustls::ClientConfig`
        // via `ClientConfig::builder()`, which panics when both `ring` and `aws_lc_rs`
        // are enabled in the unified workspace build (see PR #878). Install the Ring
        // provider once so the auto-detect has a default to find.
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            if rustls::crypto::ring::default_provider()
                .install_default()
                .is_err()
            {
                tracing::debug!(
                    "rustls crypto provider was already installed by another caller; leaving it in place"
                );
            }
        });
        Ok(Self {
            inner: Self::create_endpoint(url, user_agent)?.connect_lazy(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: `ClientTlsConfig::with_webpki_roots` invokes
    /// `rustls::ClientConfig::builder()`, which panics when both `ring` and
    /// `aws_lc_rs` are enabled in the unified workspace build. `GrpcClient::new`
    /// must install a provider before that happens.
    #[tokio::test]
    async fn grpc_client_new_does_not_panic_on_dual_provider_build() {
        let _ =
            GrpcClient::new("https://example.invalid:443", "test-agent").expect("construct lazily");
    }
}
