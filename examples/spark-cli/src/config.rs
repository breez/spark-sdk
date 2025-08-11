use std::path::PathBuf;

use bip39::Mnemonic;
use serde::{Deserialize, Serialize};
use spark_wallet::SparkWalletConfig;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub mempool_url: String,
    pub mempool_username: String,
    pub mempool_password: String,
    pub log_filter: String,
    pub log_path: PathBuf,
    pub mnemonic: Mnemonic,
    pub passphrase: String,
    pub spark_config: SparkWalletConfig,
}

pub const DEFAULT_CONFIG: &str = r#"
mempool_url: "https://regtest-mempool.loadtest.dev.sparkinfra.net/api"
mempool_username: "spark-sdk"
mempool_password: "mCMk1JqlBNtetUNy"
log_filter: "spark_wallet=debug,spark=debug,info"
log_path: "spark.log"
passphrase: ""
spark_config:
  network: "regtest"
  split_secret_threshold: 2
  operator_pool:
    coordinator_index: 0
    operators:
      - 
        id: 0
        identifier: 0000000000000000000000000000000000000000000000000000000000000001
        address: https://0.spark.loadtest.dev.sparkinfra.net/
        identity_public_key: 03d8d2d331e07f572636dfd371a30dfa139a8bdc99ea98f1f48e27dcc664589ecc
      -
        id: 1
        identifier: 0000000000000000000000000000000000000000000000000000000000000002
        address: https://1.spark.loadtest.dev.sparkinfra.net/
        identity_public_key: 023b1f3e062137ffc541a8edeaab7a4648aafa506d0208956123507d66d3886ac6
      -
        id: 2
        identifier: 0000000000000000000000000000000000000000000000000000000000000003
        address: https://2.spark.loadtest.dev.sparkinfra.net/
        identity_public_key: 02a2c62aa3230d9a51759b3d67399f57223455656369d28120fb39ef062b4469c8
  reconnect_interval_seconds: 1
  service_provider_config:
    base_url: "https://api.loadtest.dev.sparkinfra.net"
    schema_endpoint: "graphql/spark/rc"
    identity_public_key: "03e23a4912c275d1ba8742cfdfc7e9befdc2243a74be2412b7b77d227643353a1f"
  tokens_config:
    expected_withdraw_bond_sats: 10000
    expected_withdraw_relative_block_locktime: 1000
    transaction_validity_duration_seconds: 180
"#;
