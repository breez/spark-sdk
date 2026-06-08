//! Session persistence test against the MySQL `SessionStore`.
//!
//! Mirrors `session_persistence.rs`, swapping the testcontainers backend
//! from `Postgres` to `Mysql`. Both delegate to the shared scenario in
//! `breez_sdk_itest::run_session_persistence_across_restart`, so any
//! change to the workflow runs against both backends automatically.

use anyhow::Result;
use breez_sdk_itest::*;
use mysql_async::prelude::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mysql::Mysql;

struct MysqlSessionPersistenceFixture {
    /// MySQL container — must be kept alive for the test duration.
    #[allow(dead_code)]
    mysql_container: ContainerAsync<Mysql>,
    connection_string: String,
    shared_seed: [u8; 32],
    /// 33-byte serialized identity public key derived from `shared_seed`.
    identity: Vec<u8>,
}

impl MysqlSessionPersistenceFixture {
    async fn new() -> Result<Self> {
        let mysql_container = Mysql::default()
            .start()
            .await
            .expect("Failed to start MySQL container");

        let host_port = mysql_container
            .get_host_port_ipv4(3306)
            .await
            .expect("Failed to get host port");

        let connection_string = format!("mysql://root@127.0.0.1:{host_port}/test");

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
            mysql_container,
            connection_string,
            shared_seed,
            identity,
        })
    }

    async fn build_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_mysql(&self.connection_string, self.shared_seed).await
    }

    async fn read_sessions(&self) -> Result<Vec<SessionRow>> {
        let pool = mysql_async::Pool::from_url(self.connection_string.as_str())?;
        let mut conn = pool.get_conn().await?;
        let rows: Vec<(Vec<u8>, String, i64)> = conn
            .exec(
                "SELECT service_identity_key, token, expiration FROM brz_sessions \
                 WHERE user_id = ? \
                 ORDER BY service_identity_key",
                (self.identity.clone(),),
            )
            .await?;
        drop(conn);
        pool.disconnect().await?;

        let sessions = rows
            .into_iter()
            .map(|(service_identity_key, token, expiration)| SessionRow {
                service_identity_key,
                token,
                expiration: u64::try_from(expiration).unwrap_or_default(),
            })
            .collect();
        Ok(sessions)
    }

    async fn clear_sessions(&self) -> Result<()> {
        let pool = mysql_async::Pool::from_url(self.connection_string.as_str())?;
        let mut conn = pool.get_conn().await?;
        conn.exec_drop(
            "DELETE FROM brz_sessions WHERE user_id = ?",
            (self.identity.clone(),),
        )
        .await?;
        drop(conn);
        pool.disconnect().await?;
        Ok(())
    }
}

#[test_log::test(tokio::test)]
async fn test_mysql_session_persistence_across_restart() -> Result<()> {
    let fixture = MysqlSessionPersistenceFixture::new().await?;
    run_session_persistence_across_restart(
        || fixture.build_instance(),
        || fixture.read_sessions(),
        || fixture.clear_sessions(),
    )
    .await
}
