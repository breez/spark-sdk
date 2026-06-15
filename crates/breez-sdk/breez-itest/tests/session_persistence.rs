//! Session persistence test against the PostgreSQL `SessionStore`.
//!
//! Builds two SDK instances bound to the same Postgres testcontainer with a
//! shared seed and delegates to the backend-agnostic scenario in
//! `breez_sdk_itest::run_session_persistence_across_restart`. Verifies that
//! a restart picks up the cached SSP/SO sessions instead of re-running the
//! challenge-response handshake. The MySQL variant in
//! `session_persistence_mysql.rs` runs the exact same workflow.

use anyhow::Result;
use breez_sdk_itest::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

struct SessionPersistenceFixture {
    /// PostgreSQL container — must be kept alive for the test duration.
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
    connection_string: String,
    shared_seed: [u8; 32],
    /// 33-byte serialized identity public key derived from `shared_seed`.
    /// The persistent session store scopes every row by this `user_id`.
    identity: Vec<u8>,
}

impl SessionPersistenceFixture {
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

        let identity = breez_sdk_spark::identity_public_key(
            &shared_seed,
            breez_sdk_spark::Network::Regtest.into(),
            None,
        )?
        .serialize()
        .to_vec();

        Ok(Self {
            pg_container,
            connection_string,
            shared_seed,
            identity,
        })
    }

    async fn build_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_postgres(&self.connection_string, self.shared_seed).await
    }

    async fn read_sessions(&self) -> Result<Vec<SessionRow>> {
        let (client, connection) =
            tokio_postgres::connect(&self.connection_string, tokio_postgres::NoTls).await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("Postgres connection error: {e}");
            }
        });

        let rows = client
            .query(
                "SELECT service_identity_key, token, expiration FROM brz_sessions \
                 WHERE user_id = $1 \
                 ORDER BY service_identity_key",
                &[&self.identity],
            )
            .await?;

        let sessions = rows
            .into_iter()
            .map(|row| {
                let service_identity_key: Vec<u8> = row.get(0);
                let token: String = row.get(1);
                let expiration: i64 = row.get(2);
                SessionRow {
                    service_identity_key,
                    token,
                    expiration: u64::try_from(expiration).unwrap_or_default(),
                }
            })
            .collect();
        Ok(sessions)
    }

    async fn clear_sessions(&self) -> Result<()> {
        let (client, connection) =
            tokio_postgres::connect(&self.connection_string, tokio_postgres::NoTls).await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("Postgres connection error: {e}");
            }
        });
        client
            .execute(
                "DELETE FROM brz_sessions WHERE user_id = $1",
                &[&self.identity],
            )
            .await?;
        Ok(())
    }
}

#[test_log::test(tokio::test)]
async fn test_session_persistence_across_restart() -> Result<()> {
    let fixture = SessionPersistenceFixture::new().await?;
    run_session_persistence_across_restart(
        || fixture.build_instance(),
        || fixture.read_sessions(),
        || fixture.clear_sessions(),
    )
    .await
}
