use std::{fs::canonicalize, path::PathBuf};

use bip39::Mnemonic;
use bitcoin::{Address, Transaction, consensus::encode::deserialize_hex, params::Params};
use clap::Parser;
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
    pub mempool_url: String,
    pub mempool_username: String,
    pub mempool_password: String,
    pub log_filter: String,
    pub log_path: PathBuf,
    pub mnemonic: Mnemonic,
    pub passphrase: String,
    pub spark_config: SparkWalletConfig,
}
const DEFAULT_CONFIG: &str = r#"
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
        address: https://2.spark.lightspark.com
        identity_public_key: 022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410

  service_provider_config:
    base_url: "https://api.lightspark.com"    
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

    let seed = config.mnemonic.to_seed(config.passphrase.clone());
    let network = config.spark_config.network.clone();
    let signer = DefaultSigner::new(&seed, network)?;
    let wallet = spark_wallet::SparkWallet::new(config.spark_config.clone(), signer).await?;
    wallet.load().await?;
    match args.command {
        command::Command::ClaimDeposit { txid } => {
            println!("1");
            let tx = get_transaction(&config, txid).await?;
            println!("2");
            // TODO: Look for correct output index
            let leaves = wallet.claim_deposit(tx, 0).await?;
            println!("3");
            println!(
                "Claimed deposit: {}",
                serde_json::to_string_pretty(&leaves)?
            );
        }
        command::Command::GenerateDepositAddress => {
            let address = wallet.generate_deposit_address(false).await?;
            println!("{}", address);
        }
        command::Command::GenerateAndClaimDeposit => {
            let address = wallet.generate_deposit_address(false).await?;
            println!("{}", address);
            let mut rl = rustyline::DefaultEditor::new().unwrap();
            println!("Get funds from the faucet at https://app.lightspark.com/regtest-faucet");
            let line = rl.readline("paste txid> ")?;
            let txid = line.trim();
            let tx = get_transaction(&config, txid.to_string()).await?;
            let params: Params = config.spark_config.network.into();
            for (vout, output) in tx.output.iter().enumerate() {
                let Ok(output_address) = Address::from_script(&output.script_pubkey, &params)
                else {
                    continue;
                };

                if output_address != address {
                    continue;
                }

                let leaves = wallet.claim_deposit(tx, vout as u32).await?;
                println!(
                    "Claimed deposit: {}",
                    serde_json::to_string_pretty(&leaves)?
                );
                break;
            }
        }
        command::Command::PayLightningInvoice {
            invoice,
            max_fee_sat,
        } => {
            let payment = wallet.pay_lightning_invoice(&invoice, max_fee_sat).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        command::Command::CreateLightningInvoice {
            amount_sat,
            description,
        } => {
            let payment = wallet
                .create_lightning_invoice(amount_sat, description)
                .await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        command::Command::FetchLightningSendPayment { id } => {
            let payment = wallet.fetch_lightning_send_payment(&id).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        command::Command::FetchLightningReceivePayment { id } => {
            let payment = wallet.fetch_lightning_receive_payment(&id).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        command::Command::FetchLightningSendFeeEstimate { invoice } => {
            let fee = wallet.fetch_lightning_send_fee_estimate(&invoice).await?;
            println!("{}", fee);
        }
        command::Command::ListLeaves => {
            let leaves = wallet.list_leaves().await?;
            println!("{}", serde_json::to_string_pretty(&leaves)?);
        }
        command::Command::SparkAddress => {
            let spark_address = wallet.get_spark_address().await?;
            println!("{}", spark_address.to_address_string()?);
        }
        command::Command::Transfer {
            amount_sat,
            receiver_address,
        } => {
            let result = wallet.transfer(amount_sat, &receiver_address).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(())
}

async fn get_transaction(
    config: &Config,
    txid: String,
) -> Result<Transaction, Box<dyn std::error::Error>> {
    let url = format!("{}/tx/{}/hex", config.mempool_url, txid);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .basic_auth(
            config.mempool_username.clone(),
            Some(config.mempool_password.clone()),
        )
        .send()
        .await?;
    let hex = response.text().await?;
    let tx = deserialize_hex(&hex)?;
    Ok(tx)
}
