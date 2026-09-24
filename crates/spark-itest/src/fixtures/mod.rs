pub mod bitcoind;
pub mod keyshares;
pub mod ldk_server;
pub mod log;
pub mod setup;
pub mod spark_so;
pub mod sspd;
pub mod state_snapshot;
pub mod wait_log;

use anyhow::{Context, Result};
use testcontainers::core::ContainerPort;
use testcontainers::{ContainerAsync, Image};

/// Retries for up to 30 seconds: a running container can briefly inspect as having
/// no host port binding, and an exited one never gets one.
pub async fn published_port<I: Image>(container: &ContainerAsync<I>, port: u16) -> Result<u16> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match container.get_host_port_ipv4(ContainerPort::Tcp(port)).await {
            Ok(host_port) => return Ok(host_port),
            Err(e) if std::time::Instant::now() < deadline => {
                tracing::debug!("no host binding for {port} yet ({e}), retrying");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("docker never published a host port for {port}: the container is most likely not running")
                });
            }
        }
    }
}
