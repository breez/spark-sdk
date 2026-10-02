use anyhow::{Context, Result};
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio_postgres::NoTls;
use tracing::{Instrument, debug_span, instrument, warn};

use crate::fixtures::setup::FixtureId;
use crate::fixtures::spark_so::NUM_OPERATORS;
use crate::fixtures::{Container, state_snapshot};

pub const USER: &str = "postgres";
pub const PASSWORD: &str = "postgres";
pub const PORT: u16 = 5432;
pub const SSPD_DATABASE: &str = "sspd";

const READY: &str = "database system is ready to accept connections";

/// A test database can be lost, so it runs without the costs of durability.
const SETTINGS: [&str; 8] = [
    "-c",
    "fsync=off",
    "-c",
    "synchronous_commit=off",
    "-c",
    "full_page_writes=off",
    "-c",
    "max_connections=500",
];

pub fn operator_database(index: usize) -> String {
    format!("operator_{index}")
}

fn databases() -> Vec<String> {
    (0..NUM_OPERATORS)
        .map(operator_database)
        .chain([SSPD_DATABASE.to_string()])
        .collect()
}

/// The cluster's Postgres server, holding a database for each operator and one
/// for the daemon.
pub struct DatabaseFixture {
    /// Held so the server is removed when the cluster is.
    _container: Container<GenericImage>,
    pub host_name: String,
    host_port: u16,
}

impl DatabaseFixture {
    /// With `restored`, the databases hold the state snapshot; otherwise they are
    /// empty.
    #[instrument(level = "debug", name = "database.start", skip(fixture_id))]
    pub async fn start(fixture_id: &FixtureId, restored: bool) -> Result<Self> {
        let host_name = format!("postgres-{fixture_id}");
        let image = if restored {
            // The snapshot's server starts without initializing, so it reports
            // ready once, where a fresh one reports it for a temporary server too.
            let (name, tag) = state_snapshot::database_image().await?;
            GenericImage::new(name, tag).with_wait_for(WaitFor::message_on_stderr(READY))
        } else {
            GenericImage::new("postgres", "11-alpine")
                .with_wait_for(WaitFor::message_on_stdout(READY))
                .with_wait_for(WaitFor::message_on_stderr(READY))
        };
        let container = image
            .with_exposed_port(ContainerPort::Tcp(PORT))
            .with_network(fixture_id.to_network())
            .with_container_name(&host_name)
            .with_env_var("POSTGRES_PASSWORD", PASSWORD)
            .with_cmd(std::iter::once("postgres").chain(SETTINGS))
            .start()
            .instrument(debug_span!("database.container"))
            .await
            .context("starting the cluster's postgres")?;
        let host_port = crate::fixtures::published_port(&container, PORT).await?;
        let database = Self {
            _container: Container::new(container),
            host_name,
            host_port,
        };
        if !restored {
            database.create_databases().await?;
        }
        Ok(database)
    }

    async fn create_databases(&self) -> Result<()> {
        let (client, connection) = tokio_postgres::connect(&self.host_url("postgres"), NoTls)
            .await
            .context("connecting to the cluster's postgres")?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                warn!("database connection error: {e}");
            }
        });
        for database in databases() {
            client
                .batch_execute(&format!("CREATE DATABASE {database}"))
                .await
                .with_context(|| format!("creating database {database}"))?;
        }
        Ok(())
    }

    /// The URL another container on the cluster's network reaches `database` at.
    pub fn internal_url(&self, database: &str) -> String {
        format!(
            "postgres://{USER}:{PASSWORD}@{}:{PORT}/{database}?sslmode=disable",
            self.host_name
        )
    }

    /// The URL the test process reaches `database` at.
    pub fn host_url(&self, database: &str) -> String {
        format!(
            "postgres://{USER}:{PASSWORD}@127.0.0.1:{}/{database}?sslmode=disable",
            self.host_port
        )
    }
}
