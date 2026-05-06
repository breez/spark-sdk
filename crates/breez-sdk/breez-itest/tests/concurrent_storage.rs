//! Multi-instance concurrent storage integration test against PostgreSQL.
//!
//! Spins up a PostgreSQL `testcontainer`, builds three SDK instances bound to
//! the same database with a shared seed, then delegates the actual workflow to
//! the backend-agnostic scenario in
//! `breez_sdk_itest::run_concurrent_multi_instance_operations`. The MySQL
//! variant in `concurrent_storage_mysql.rs` runs the exact same workflow.
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                   PostgreSQL Container                   │
//! │                    (testcontainers)                      │
//! └────────────┬──────────────┬──────────────┬──────────────┘
//!              │              │              │
//!       ┌──────▼──────┐ ┌─────▼──────┐ ┌─────▼──────┐
//!       │ Instance 0  │ │ Instance 1 │ │ Instance 2 │
//!       │ (seed A)    │ │ (seed A)   │ │ (seed A)   │
//!       │ own pool    │ │ own pool   │ │ own pool   │
//!       └──────┬──────┘ └─────┬──────┘ └─────┬──────┘
//!              │              │              │
//!              └──────────────┼──────────────┘
//!                             │ Spark transfers
//!                       ┌─────▼──────┐
//!                       │ Counterparty│
//!                       │ (seed B)    │
//!                       │ SQLite      │
//!                       └─────────────┘
//! ```

use anyhow::Result;
use breez_sdk_itest::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

/// Test fixture for the postgres-backed concurrent test.
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
        build_sdk_with_postgres(&self.connection_string, self.shared_seed).await
    }
}

#[test_log::test(tokio::test)]
async fn test_concurrent_multi_instance_operations() -> Result<()> {
    let fixture = ConcurrentTestFixture::new().await?;
    run_concurrent_multi_instance_operations(|| fixture.build_instance()).await
}
