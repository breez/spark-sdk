//! Multi-instance concurrent storage integration test against MySQL.
//!
//! Mirrors `concurrent_storage.rs`, swapping the testcontainers backend from
//! `Postgres` to `Mysql`. Both delegate to the shared scenario in
//! `breez_sdk_itest::run_concurrent_multi_instance_operations`, so any change
//! to the workflow runs against both backends automatically.

use anyhow::Result;
use breez_sdk_itest::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mysql::Mysql;

struct MysqlConcurrentTestFixture {
    /// MySQL container — must be kept alive for the test duration.
    #[allow(dead_code)]
    mysql_container: ContainerAsync<Mysql>,
    connection_string: String,
    shared_seed: [u8; 32],
}

impl MysqlConcurrentTestFixture {
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

        Ok(Self {
            mysql_container,
            connection_string,
            shared_seed,
        })
    }

    async fn build_instance(&self) -> Result<SdkInstance> {
        build_sdk_with_mysql(&self.connection_string, self.shared_seed).await
    }
}

#[test_log::test(tokio::test)]
async fn test_mysql_concurrent_multi_instance_operations() -> Result<()> {
    let fixture = MysqlConcurrentTestFixture::new().await?;
    run_concurrent_multi_instance_operations(|| fixture.build_instance()).await
}
