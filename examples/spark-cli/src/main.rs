use std::{fs::canonicalize, path::PathBuf};

use bip39::Mnemonic;
use clap::Parser;
use electrum_client::{
    ElectrumApi,
    bitcoin::{Address, address::NetworkUnchecked},
};
use figment::{
    Figment,
    providers::{Env, Format, Yaml},
};
use serde::{Deserialize, Serialize};
use spark_wallet::{DefaultSigner, SparkWalletConfig};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod command;

#[derive(Clone, Debug, Parser)]
struct Args {
    /// Config path, relative to the working directory.
    #[arg(long, default_value = "spark.conf")]
    pub config: PathBuf,

    /// Working directory
    #[arg(long, default_value = ".spark")]
    pub working_directory: PathBuf,

    #[command(subcommand)]
    pub command: command::Command,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub electrum_url: String,
    pub log_filter: String,
    pub log_path: PathBuf,
    pub mnemonic: Mnemonic,
    pub passphrase: String,
    pub spark_config: SparkWalletConfig,
}
const DEFAULT_CONFIG: &str = r#"
electrum_url: "https://regtest-mempool.us-west-2.sparkinfra.net/api"
log_filter: "spark_wallet=debug,spark=debug,info"
log_path: "spark.log"
passphrase: ""
spark_config:
  network: "regtest"
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
        address: https://2.spark.lightspark.com
        identity_public_key: 022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410

  service_provider_config:
    base_url: "https://api.lightspark.com"
    schema_endpoint: ""
    identity_public_key: "022bf283544b16c0622daecb79422007d167eca6ce9f0c98c0c49833b1f7170bfe"
"#;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.working_directory)?;
    std::env::set_current_dir(&args.working_directory)?;

    let config_file = canonicalize(&args.config).ok();
    let mut figment = Figment::new().merge(Yaml::string(DEFAULT_CONFIG));
    if let Some(config_file) = &config_file {
        figment = figment.merge(Yaml::file(config_file));
    }

    let config: Config = figment.merge(Env::prefixed("SPARK_")).extract()?;

    tracing_subscriber::registry()
        .with(EnvFilter::new(&config.log_filter))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .init();

    let seed = config.mnemonic.to_seed(config.passphrase);
    let network = config.spark_config.network;
    let signer = DefaultSigner::new(
        seed.into_iter()
            .take(32)
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap(),
        network,
    )?;
    let wallet = spark_wallet::SparkWallet::new(config.spark_config, signer).await?;
    match args.command {
        command::Command::ClaimDeposit { address } => {
            let address: Address<NetworkUnchecked> = address.parse()?;
            let address = address.require_network(network.try_into()?)?;
            let electrum_client = electrum_client::Client::new(&config.electrum_url)?;
            let unspent = electrum_client.script_list_unspent(&address.script_pubkey())?;
            if unspent.is_empty() {
                println!("No unspent outputs found for address: {}", address);
                return Ok(());
            }

            if unspent.len() > 1 {
                println!("Multiple unspent outputs found for address: {}", address);
                return Ok(());
            }

            let unspent = unspent.into_iter().nth(0).unwrap();
            let tx = electrum_client.transaction_get(&unspent.tx_hash)?;
            let leaves = wallet.claim_deposit(tx, unspent.tx_pos as u32).await?;
            println!(
                "Claimed deposit: {}",
                serde_json::to_string_pretty(&leaves)?
            );
        }
        command::Command::GenerateDepositAddress => {
            let address = wallet.generate_deposit_address(true).await?;
            println!("{}", address);
        }
    }

    Ok(())
}
