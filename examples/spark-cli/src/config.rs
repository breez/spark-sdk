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
mempool_url: "https://regtest-mempool.us-west-2.sparkinfra.net/api"
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
        address: https://0.spark.lightspark.com
        identity_public_key: 03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763
      -
        id: 1
        identifier: 0000000000000000000000000000000000000000000000000000000000000002
        address: https://1.spark.lightspark.com
        identity_public_key: 03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77
      -
        id: 2
        identifier: 0000000000000000000000000000000000000000000000000000000000000003
        address: https://2.spark.flashnet.xyz
        identity_public_key: 022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410

  service_provider_config:
    base_url: "https://api.lightspark.com"    
    identity_public_key: "022bf283544b16c0622daecb79422007d167eca6ce9f0c98c0c49833b1f7170bfe"
"#;
