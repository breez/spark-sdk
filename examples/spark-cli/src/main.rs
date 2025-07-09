use std::{fs::canonicalize, path::PathBuf};

use clap::Parser;
use figment::{
    Figment,
    providers::{Env, Format, Yaml},
};
use spark_wallet::DefaultSigner;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{Config, DEFAULT_CONFIG};

mod command;
mod config;
mod deposit;
mod leaves;
mod lightning;
mod transfer;

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
    wallet.sync().await?;
    match args.command {
        command::Command::Balance => {
            let balance = wallet.get_balance().await?;
            println!("Balance: {} sats", balance);
        }
        command::Command::Deposit(deposit_command) => {
            deposit::handle_command(&config, &wallet, deposit_command).await?
        }
        command::Command::Info => {
            let info = wallet.get_info().await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        command::Command::Leaves(leaves_command) => {
            leaves::handle_command(&config, &wallet, leaves_command).await?
        }
        command::Command::Lightning(lightning_command) => {
            lightning::handle_command(&config, &wallet, lightning_command).await?
        }
        command::Command::SparkAddress => {
            let spark_address = wallet.get_spark_address().await?;
            println!("{}", spark_address.to_address_string()?);
        }
        command::Command::Sync => {
            wallet.sync().await?;
            println!("Wallet synced successfully.");
        }
        command::Command::Transfer(transfer_command) => {
            transfer::handle_command(&config, &wallet, transfer_command).await?
        }
    }
    Ok(())
}
