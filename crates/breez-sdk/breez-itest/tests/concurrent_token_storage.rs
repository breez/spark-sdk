//! Concurrent token storage stress test against PostgreSQL.
//!
//! Spins up a PostgreSQL `testcontainer`, builds three SDK instances bound to
//! the same database with a shared seed, then delegates to the backend-agnostic
//! scenario in `breez_sdk_itest::run_concurrent_token_operations`. The MySQL
//! variant in `concurrent_token_storage_mysql.rs` runs the exact same workflow.
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
//!       │ issuer      │ │ syncer     │ │ syncer     │
//!       └──────┬──────┘ └─────┬──────┘ └─────┬──────┘
//!              │              │              │
//!              └──────────────┼──────────────┘
//!                             │ token payments (bidirectional)
//!                       ┌─────▼──────┐
//!                       │    Bob     │
//!                       │ (seed B)   │
//!                       │ SQLite     │
//!                       └────────────┘
//! ```

use anyhow::Result;
use breez_sdk_itest::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

struct ConcurrentTestFixture {
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
async fn test_concurrent_token_operations() -> Result<()> {
    let fixture = ConcurrentTestFixture::new().await?;
    run_concurrent_token_operations(|| fixture.build_instance()).await
}
