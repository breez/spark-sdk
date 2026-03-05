use crate::fixtures::docker::{DockerImageConfig, build_docker_image};
use anyhow::Result;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Host, WaitFor, wait::LogWaitStrategy},
    runners::AsyncRunner,
};
use testcontainers_modules::postgres::Postgres;
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
    /// Database URL. Use `:memory:` for in-memory SQLite or a
    /// `postgres://...` URL for PostgreSQL.
    pub db_url: String,
    /// Whether to include Spark address routing hints in Lightning invoices
    pub include_spark_address: bool,
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
            db_url: ":memory:".to_string(),
            include_spark_address: false,
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

    /// Set the database URL (`:memory:` for SQLite, `postgres://...` for PostgreSQL)
    pub fn with_db_url(mut self, db_url: impl Into<String>) -> Self {
        self.db_url = db_url.into();
        self
    }

    /// Set whether to include Spark address routing hints in invoices
    pub fn with_include_spark_address(mut self, include_spark_address: bool) -> Self {
        self.include_spark_address = include_spark_address;
        self
    }
}

pub struct LnurlFixture {
    pub container: ContainerAsync<GenericImage>,
    pub http_url: String,
    /// Keeps the PostgreSQL container alive when the LNURL server uses a
    /// Postgres backend. `None` when using SQLite.
    #[allow(dead_code)]
    pg_container: Option<ContainerAsync<Postgres>>,
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
        .with_env_var("BREEZ_LNURL_DB_URL", &config.db_url);

        // Add additional test-friendly defaults
        container = container
            .with_env_var("BREEZ_LNURL_LOG_LEVEL", "lnurl=trace,info")
            // Allow all domains for testing (empty string means no domain validation)
            .with_env_var("BREEZ_LNURL_DOMAINS", "")
            // Use HTTP scheme for testing
            .with_env_var("BREEZ_LNURL_SCHEME", "http")
            // Set a test nostr secret key for zap receipt support (nsec encoding of key 0x00...01)
            .with_env_var(
                "BREEZ_LNURL_NSEC",
                "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsmhltgl",
            )
            .with_env_var("BREEZ_LNURL_MIN_SENDABLE", "1000")
            .with_env_var("BREEZ_LNURL_MAX_SENDABLE", "1000000000")
            .with_env_var(
                "BREEZ_LNURL_INCLUDE_SPARK_ADDRESS",
                config.include_spark_address.to_string(),
            )
            // Allow the container to reach services on the host via host.docker.internal
            .with_host("host.docker.internal", Host::HostGateway);

        let container = container.start().await?;

        info!("Lnurl container running");

        // Get the host ports
        let host_http_port = container.get_host_port_ipv4(HTTP_PORT).await?;

        let http_url = format!("http://127.0.0.1:{host_http_port}");

        info!("Lnurl service available at HTTP: {http_url}");

        Ok(Self {
            container,
            http_url,
            pg_container: None,
        })
    }

    /// Create a new LnurlFixture backed by a PostgreSQL database.
    ///
    /// Starts a Postgres testcontainer, then launches the LNURL server
    /// configured to connect to it via `host.docker.internal`.
    pub async fn new_with_postgres() -> Result<Self> {
        let pg_container = Postgres::default()
            .start()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start PostgreSQL container: {e}"))?;

        let host_port = pg_container
            .get_host_port_ipv4(5432)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get PostgreSQL host port: {e}"))?;

        // The LNURL server runs inside Docker, so it reaches PostgreSQL
        // via the host gateway.
        let db_url =
            format!("postgres://postgres:postgres@host.docker.internal:{host_port}/postgres");

        info!("Starting LNURL server with PostgreSQL backend at port {host_port}");
        let config = LnurlImageConfig::default().with_db_url(db_url);
        let mut fixture = Self::with_config(config).await?;
        fixture.pg_container = Some(pg_container);
        Ok(fixture)
    }

    pub fn http_url(&self) -> &str {
        &self.http_url
    }
}
