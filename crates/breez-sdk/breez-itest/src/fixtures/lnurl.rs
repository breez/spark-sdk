use crate::fixtures::docker::{DockerImageConfig, build_docker_image};
use anyhow::Result;
use spark_wallet::{SparkDeployment, SparkWalletConfig};
use std::time::Duration;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Host, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};
use testcontainers_modules::postgres::Postgres;
use tracing::info;

const HTTP_PORT: u16 = 8080;
const SPARK_CONFIG_PATH: &str = "/spark-config.json";

/// Configuration for building the lnurl Docker image
#[derive(Debug, Clone)]
pub struct LnurlImageConfig {
    /// Base Docker image config
    pub docker_config: DockerImageConfig,
    /// Network to use (mainnet, regtest)
    pub network: String,
    /// Whether to auto-migrate the database
    pub auto_migrate: bool,
    /// Whether to include Spark address routing hints in Lightning invoices
    pub include_spark_address: bool,
    /// Spark deployment to issue invoices through instead of the network's.
    pub spark_deployment: Option<SparkDeployment>,
    /// Partner JWT the server serves as every domain's own.
    pub partner_jwt: Option<String>,
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
                build_args: vec![("CARGO_FEATURES".to_string(), "dev".to_string())],
            },
            network: "regtest".to_string(),
            auto_migrate: true,
            include_spark_address: false,
            spark_deployment: None,
            partner_jwt: None,
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

    /// Set whether to include Spark address routing hints in invoices
    pub fn with_include_spark_address(mut self, include_spark_address: bool) -> Self {
        self.include_spark_address = include_spark_address;
        self
    }

    /// Issue invoices through `deployment`, reached from the container like
    /// any other host, instead of the network's operators and SSP.
    pub fn with_spark_deployment(mut self, deployment: SparkDeployment) -> Self {
        self.spark_deployment = Some(deployment);
        self
    }

    /// Serve `jwt` as every domain's partner JWT, so the server attributes and
    /// can quote receives off mainnet.
    pub fn with_partner_jwt(mut self, jwt: impl Into<String>) -> Self {
        self.partner_jwt = Some(jwt.into());
        self
    }
}

pub struct LnurlFixture {
    pub container: ContainerAsync<GenericImage>,
    pub http_url: String,
    /// Connection string to the server's database, from the host.
    db_url: String,
    /// Keeps the PostgreSQL container the server is pointed at alive for the
    /// lifetime of the fixture.
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
}

impl LnurlFixture {
    /// Create a new LnurlFixture using the default configuration
    /// This builds the image from the local lnurl crate
    pub async fn new() -> Result<Self> {
        Self::with_config(LnurlImageConfig::default()).await
    }

    /// Create a new LnurlFixture with a custom configuration.
    ///
    /// Starts a PostgreSQL testcontainer and points the LNURL server at it:
    /// postgres is the server's only supported database.
    pub async fn with_config(config: LnurlImageConfig) -> Result<Self> {
        Self::start(config, None).await
    }

    /// An LNURL server on a local cluster's docker network, using the operators and
    /// SSP in `spark_config` instead of the network's.
    pub async fn on_local_cluster(
        docker_network: &str,
        spark_config: &SparkWalletConfig,
    ) -> Result<Self> {
        Self::start(
            LnurlImageConfig::default(),
            Some((docker_network, spark_config)),
        )
        .await
    }

    async fn start(
        config: LnurlImageConfig,
        local_cluster: Option<(&str, &SparkWalletConfig)>,
    ) -> Result<Self> {
        // Build the Docker image from the context
        build_docker_image(&config.docker_config).await?;

        let pg_container = Postgres::default()
            .start()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start PostgreSQL container: {e}"))?;
        let pg_port = pg_container
            .get_host_port_ipv4(5432)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get PostgreSQL host port: {e}"))?;
        // The LNURL server runs inside Docker, so it reaches PostgreSQL
        // via the host gateway.
        let db_url =
            format!("postgres://postgres:postgres@host.docker.internal:{pg_port}/postgres");
        info!("Starting LNURL server with PostgreSQL backend at port {pg_port}");

        // Create container from the built image with environment variables
        let mut container = GenericImage::new(
            &config.docker_config.image_name,
            &config.docker_config.image_tag,
        )
        .with_exposed_port(ContainerPort::Tcp(HTTP_PORT))
        // Not the startup log line: the server prints it before it migrates
        // the database and binds, and logs nothing after, so waiting on it
        // hands out a URL that nothing is listening on yet.
        .with_wait_for(WaitFor::http(
            HttpWaitStrategy::new("/health")
                .with_port(ContainerPort::Tcp(HTTP_PORT))
                .with_expected_status_code(200_u16),
        ))
        .with_log_consumer(crate::log::TracingConsumer::new("lnurl"))
        .with_env_var("BREEZ_LNURL_NETWORK", &config.network)
        .with_env_var("BREEZ_LNURL_AUTO_MIGRATE", config.auto_migrate.to_string())
        .with_env_var("BREEZ_LNURL_DB_URL", &db_url);

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
                "BREEZ_LNURL_DEV_DONT_USE_LNURL_INCLUDE_SPARK_ADDRESS",
                config.include_spark_address.to_string(),
            )
            // Allow the container to reach services on the host via host.docker.internal
            .with_host("host.docker.internal", Host::HostGateway);
        let spark_config = match (local_cluster, &config.spark_deployment) {
            (Some((docker_network, spark_config)), _) => {
                container = container.with_network(docker_network);
                Some(serde_json::to_vec(spark_config)?)
            }
            (None, Some(deployment)) => Some(serde_json::to_vec(deployment)?),
            (None, None) => None,
        };
        if let Some(spark_config) = spark_config {
            container = container
                .with_copy_to(SPARK_CONFIG_PATH, spark_config)
                .with_env_var("BREEZ_LNURL_SPARK_CONFIG", SPARK_CONFIG_PATH);
        }
        if let Some(jwt) = &config.partner_jwt {
            container = container.with_env_var("BREEZ_LNURL_DEV_DONT_USE_PARTNER_JWT", jwt);
        }

        let container = container.start().await?;

        info!("Lnurl container running");

        // Get the host ports
        let host_http_port = container.get_host_port_ipv4(HTTP_PORT).await?;

        let http_url = format!("http://127.0.0.1:{host_http_port}");

        info!("Lnurl service available at HTTP: {http_url}");

        Ok(Self {
            container,
            http_url,
            db_url: format!("postgres://postgres:postgres@127.0.0.1:{pg_port}/postgres"),
            pg_container,
        })
    }

    pub fn http_url(&self) -> &str {
        &self.http_url
    }

    /// The domain the server serves this fixture's addresses under: the host
    /// and port of [`Self::http_url`].
    pub fn domain(&self) -> &str {
        self.http_url
            .strip_prefix("http://")
            .unwrap_or(&self.http_url)
    }

    /// A client for the server's database.
    pub async fn db(&self) -> Result<tokio_postgres::Client> {
        let (client, connection) =
            tokio_postgres::connect(&self.db_url, tokio_postgres::NoTls).await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::warn!("LNURL database connection closed: {e}");
            }
        });
        Ok(client)
    }

    /// Allows [`Self::domain`] and switches its partner fee quoting on or off,
    /// then waits for the server to load the change. The server reads its
    /// domains once a minute, so this takes up to a minute.
    pub async fn set_fee_quoting(&self, on: bool) -> Result<()> {
        let domain = self.domain();
        let logged_before = self.container.stdout_to_vec().await?.len();
        let db = self.db().await?;
        db.execute(
            "INSERT INTO allowed_domains (domain) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&domain],
        )
        .await?;
        db.execute(
            "INSERT INTO domain_attribution (domain, api_key, quote_receives)
             VALUES ($1, 'unused-api-key', $2)
             ON CONFLICT (domain) DO UPDATE SET quote_receives = excluded.quote_receives",
            &[&domain, &on],
        )
        .await?;

        // What the server logs when it loads the change: a new domain, or a
        // known one whose quoting switched.
        let switched = if on { "enabled" } else { "disabled" };
        let markers = [
            format!("allowed domain added: {domain}"),
            format!("partner fee quoting {switched} for domain: {domain}"),
        ];
        for _ in 0..90 {
            let logs = self.container.stdout_to_vec().await?;
            let new_logs = String::from_utf8_lossy(&logs[logged_before.min(logs.len())..]);
            if markers.iter().any(|m| new_logs.contains(m.as_str())) {
                info!("LNURL server loaded fee quoting {switched} for {domain}");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        anyhow::bail!("LNURL server did not load fee quoting {switched} for {domain}")
    }
}
