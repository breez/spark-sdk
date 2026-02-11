use crate::fixtures::docker::{DockerImageConfig, build_docker_image};
use anyhow::Result;
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
};
use tracing::info;

const GRPC_PORT: u16 = 8080;

/// Configuration for building the data-sync Docker image
pub type DataSyncImageConfig = DockerImageConfig;

impl Default for DataSyncImageConfig {
    fn default() -> Self {
        Self {
            // Default to the GitHub repository
            context_path: "https://github.com/breez/data-sync.git".to_string(),
            dockerfile_path: "Dockerfile".to_string(),
            image_name: "breez-data-sync-built".to_string(),
            image_tag: "latest".to_string(),
        }
    }
}

pub struct DataSyncFixture {
    pub container: ContainerAsync<GenericImage>,
    pub grpc_url: String,
}

impl DataSyncFixture {
    /// Create a new DataSyncFixture using the default configuration.
    ///
    /// If `DATA_SYNC_PATH` is set, builds the Docker image from that local path.
    /// Otherwise, builds from the GitHub repository.
    pub async fn new() -> Result<Self> {
        let config = match std::env::var("DATA_SYNC_PATH") {
            Ok(path) => {
                info!("Using local data-sync source: {path}");
                DataSyncImageConfig {
                    context_path: path,
                    ..DataSyncImageConfig::default()
                }
            }
            Err(_) => DataSyncImageConfig::default(),
        };
        Self::with_config(config).await
    }

    /// Create a new DataSyncFixture with a custom configuration
    pub async fn with_config(config: DataSyncImageConfig) -> Result<Self> {
        // Build the Docker image from the context
        build_docker_image(&config).await?;

        // Create container from the built image
        let container = GenericImage::new(&config.image_name, &config.image_tag)
            .with_exposed_port(ContainerPort::Tcp(GRPC_PORT))
            .with_wait_for(WaitFor::message_on_stderr("Server listening at"))
            .with_log_consumer(crate::log::TracingConsumer::new("data-sync"))
            .start()
            .await?;

        info!("Data-sync container running");

        // Get the host ports
        let host_grpc_port = container.get_host_port_ipv4(GRPC_PORT).await?;

        let grpc_url = format!("http://127.0.0.1:{host_grpc_port}");

        info!("Data-sync service available at gRPC: {grpc_url}",);

        Ok(Self {
            container,
            grpc_url,
        })
    }

    pub fn grpc_url(&self) -> &str {
        &self.grpc_url
    }
}
