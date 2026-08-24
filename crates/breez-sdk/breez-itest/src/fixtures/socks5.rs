//! A real SOCKS5 proxy in a container, so proxy tests exercise the actual
//! handshake rather than a stub.

use anyhow::Result;
use breez_sdk_spark::ProxyConfig;
use testcontainers::{
    ContainerAsync, ContainerRequest, GenericImage, ImageExt,
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
};

const SOCKS_PORT: u16 = 1080;

/// A running SOCKS5 proxy. The container stops when this is dropped, so keep it
/// alive for as long as the SDK under test needs to reach the network.
pub struct Socks5Proxy {
    _container: ContainerAsync<GenericImage>,
    config: ProxyConfig,
}

impl Socks5Proxy {
    /// Starts a proxy with no authentication.
    pub async fn start() -> Result<Self> {
        Self::start_inner(None).await
    }

    /// Starts a proxy that requires the given credentials.
    pub async fn start_with_credentials(user: &str, password: &str) -> Result<Self> {
        Self::start_inner(Some((user.to_string(), password.to_string()))).await
    }

    async fn start_inner(credentials: Option<(String, String)>) -> Result<Self> {
        let mut image: ContainerRequest<GenericImage> =
            GenericImage::new("serjs/go-socks5-proxy", "latest")
                .with_exposed_port(ContainerPort::Tcp(SOCKS_PORT))
                // The image logs to stderr, including its ready line.
                .with_wait_for(WaitFor::message_on_stderr("Start listening proxy service"))
                .into();

        image = match &credentials {
            Some((user, password)) => image
                .with_env_var("PROXY_USER", user)
                .with_env_var("PROXY_PASSWORD", password),
            // The image refuses to start unless authentication is either
            // configured or explicitly waived.
            None => image.with_env_var("REQUIRE_AUTH", "false"),
        };

        let container = image.start().await?;
        let port = container.get_host_port_ipv4(SOCKS_PORT).await?;
        let (username, password) = match credentials {
            Some((user, password)) => (Some(user), Some(password)),
            None => (None, None),
        };

        Ok(Self {
            _container: container,
            config: ProxyConfig {
                // The container publishes on the host loopback, which is also
                // what a real Tor or local proxy deployment looks like.
                host: "127.0.0.1".to_string(),
                port,
                username,
                password,
            },
        })
    }

    /// The config to put on `Config::proxy`.
    pub fn config(&self) -> ProxyConfig {
        self.config.clone()
    }
}
