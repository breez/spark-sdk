//! Server-mode multi-instance concurrent storage test against PostgreSQL.
//!
//! Mirror of `concurrent_storage.rs`, but each SDK is built with
//! `default_server_config` (`background_tasks_enabled=false`). The scenario
//! body in `breez_sdk_itest::run_concurrent_multi_instance_operations` adapts
//! to the runtime mode internally — funding goes through `ensure_funded_via_polling`
//! and Scenario 3 drives an explicit 3-way sync before reading balances on all
//! instances, since server-mode `get_info(ensure_synced=true)` is a no-op and
//! the SDK doesn't auto-propagate incoming transfers across sibling instances.
//!
//! See `concurrent_storage.rs` for the architecture diagram.

use anyhow::Result;
use breez_sdk_itest::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

struct ConcurrentTestFixture {
    /// PostgreSQL container — must be kept alive for the test duration.
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
    connection_string: String,
    shared_seed: [u8; 32],
}

impl ConcurrentTestFixture {
    async fn new() -> Result<Self> {
        let pg_container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");

        let host_port = pg_container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");

        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );

        let mut shared_seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut shared_seed);

        Ok(Self {
            pg_container,
            connection_string,
            shared_seed,
        })
    }

    async fn build_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_postgres_server_mode(&self.connection_string, self.shared_seed).await
    }
}

#[test_log::test(tokio::test)]
async fn test_concurrent_multi_instance_operations_server_mode() -> Result<()> {
    let fixture = ConcurrentTestFixture::new().await?;
    run_concurrent_multi_instance_operations(RuntimeMode::Server, || fixture.build_instance()).await
}
