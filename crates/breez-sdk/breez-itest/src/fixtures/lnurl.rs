use crate::fixtures::docker::{DockerImageConfig, build_docker_image};
use anyhow::Result;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, WaitFor, wait::LogWaitStrategy},
    runners::AsyncRunner,
};
use tracing::info;

const HTTP_PORT: u16 = 8080;

/// Configuration for building the lnurl Docker image
#[derive(Debug, Clone)]
pub struct LnurlImageConfig {
    /// Base Docker image config
    pub docker_config: DockerImageConfig,
    /// Network to use (mainnet, regtest)
    pub network: String,
    /// Whether to auto-migrate the database
    pub auto_migrate: bool,
}

impl Default for LnurlImageConfig {
    fn default() -> Self {
        Self {
            docker_config: DockerImageConfig {
                // Default to workspace root (Dockerfile expects workspace context)
                context_path: "../../..".to_string(),
                dockerfile_path: "crates/breez-sdk/lnurl/Dockerfile".to_string(),
                image_name: "breez-lnurl-built".to_string(),
                image_tag: "latest".to_string(),
            },
            network: "regtest".to_string(),
            auto_migrate: true,
        }
    }
}

impl LnurlImageConfig {
    /// Create a config with a local repository path
    pub fn with_local_path(path: impl Into<String>) -> Self {
        let mut config = Self::default();
        config.docker_config.context_path = path.into();
        config
    }

    /// Set the network (mainnet, regtest)
    pub fn with_network(mut self, network: impl Into<String>) -> Self {
        self.network = network.into();
        self
    }

    /// Set auto-migration
    pub fn with_auto_migrate(mut self, auto_migrate: bool) -> Self {
        self.auto_migrate = auto_migrate;
        self
    }
}

pub struct LnurlFixture {
    pub container: ContainerAsync<GenericImage>,
    pub http_url: String,
    _config: LnurlImageConfig,
}

impl LnurlFixture {
    /// Create a new LnurlFixture using the default configuration
    /// This builds the image from the local lnurl crate
    pub async fn new() -> Result<Self> {
        Self::with_config(LnurlImageConfig::default()).await
    }

    /// Create a new LnurlFixture with a custom configuration
    pub async fn with_config(config: LnurlImageConfig) -> Result<Self> {
        // Build the Docker image from the context
        build_docker_image(&config.docker_config).await?;

        // Create container from the built image with environment variables
        let mut container = GenericImage::new(
            &config.docker_config.image_name,
            &config.docker_config.image_tag,
        )
        .with_exposed_port(ContainerPort::Tcp(HTTP_PORT))
        .with_wait_for(WaitFor::Log(LogWaitStrategy::stdout(
            "starting lnurl server",
        )))
        .with_log_consumer(crate::log::TracingConsumer::new("lnurl"))
        .with_env_var("BREEZ_LNURL_NETWORK", &config.network)
        .with_env_var("BREEZ_LNURL_AUTO_MIGRATE", config.auto_migrate.to_string())
        // Use in-memory SQLite database for testing
        .with_env_var("BREEZ_LNURL_DB_URL", ":memory:");

        // Add additional test-friendly defaults
        container = container
            // Allow all domains for testing (empty string means no domain validation)
            .with_env_var("BREEZ_LNURL_DOMAINS", "")
            // Use HTTP scheme for testing
            .with_env_var("BREEZ_LNURL_SCHEME", "http")
            .with_env_var("BREEZ_LNURL_MIN_SENDABLE", "1000")
            .with_env_var("BREEZ_LNURL_MAX_SENDABLE", "1000000000");

        let container = container.start().await?;

        info!("Lnurl container running");

        // Get the host ports
        let host_http_port = container.get_host_port_ipv4(HTTP_PORT).await?;

        let http_url = format!("http://127.0.0.1:{host_http_port}");

        info!("Lnurl service available at HTTP: {http_url}");

        Ok(Self {
            container,
            http_url,
            _config: config,
        })
    }

    pub fn http_url(&self) -> &str {
        &self.http_url
    }
}
