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
    wallet.load().await?;
    match args.command {
        command::Command::Deposit(deposit_command) => {
            deposit::handle_command(&config, &wallet, deposit_command).await?
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
        command::Command::Transfer(transfer_command) => {
            transfer::handle_command(&config, &wallet, transfer_command).await?
        }
    }
    // match args.command {
    //     command::Command::ClaimDeposit { txid } => {
    //         println!("1");
    //         let tx = get_transaction(&config, txid).await?;
    //         println!("2");
    //         // TODO: Look for correct output index
    //         let leaves = wallet.claim_deposit(tx, 0).await?;
    //         println!("3");
    //         println!(
    //             "Claimed deposit: {}",
    //             serde_json::to_string_pretty(&leaves)?
    //         );
    //     }
    //     command::Command::GenerateDepositAddress => {
    //         let address = wallet.generate_deposit_address(false).await?;
    //         println!("{}", address);
    //     }
    //     command::Command::GenerateAndClaimDeposit => {
    //         let address = wallet.generate_deposit_address(false).await?;
    //         println!("{}", address);
    //         let mut rl = rustyline::DefaultEditor::new().unwrap();
    //         println!("Get funds from the faucet at https://app.lightspark.com/regtest-faucet");
    //         let line = rl.readline("paste txid> ")?;
    //         let txid = line.trim();
    //         let tx = get_transaction(&config, txid.to_string()).await?;
    //         let params: Params = config.spark_config.network.into();
    //         for (vout, output) in tx.output.iter().enumerate() {
    //             let Ok(output_address) = Address::from_script(&output.script_pubkey, &params)
    //             else {
    //                 continue;
    //             };

    //             if output_address != address {
    //                 continue;
    //             }

    //             let leaves = wallet.claim_deposit(tx, vout as u32).await?;
    //             println!(
    //                 "Claimed deposit: {}",
    //                 serde_json::to_string_pretty(&leaves)?
    //             );
    //             break;
    //         }
    //     }
    //     command::Command::PayLightningInvoice {
    //         invoice,
    //         max_fee_sat,
    //     } => {
    //         let payment = wallet.pay_lightning_invoice(&invoice, max_fee_sat).await?;
    //         println!("{}", serde_json::to_string_pretty(&payment)?);
    //     }
    //     command::Command::CreateLightningInvoice {
    //         amount_sat,
    //         description,
    //     } => {
    //         let payment = wallet
    //             .create_lightning_invoice(amount_sat, description)
    //             .await?;
    //         println!("{}", serde_json::to_string_pretty(&payment)?);
    //     }
    //     command::Command::FetchLightningSendPayment { id } => {
    //         let payment = wallet.fetch_lightning_send_payment(&id).await?;
    //         println!("{}", serde_json::to_string_pretty(&payment)?);
    //     }
    //     command::Command::FetchLightningReceivePayment { id } => {
    //         let payment = wallet.fetch_lightning_receive_payment(&id).await?;
    //         println!("{}", serde_json::to_string_pretty(&payment)?);
    //     }
    //     command::Command::FetchLightningSendFeeEstimate { invoice } => {
    //         let fee = wallet.fetch_lightning_send_fee_estimate(&invoice).await?;
    //         println!("{}", fee);
    //     }
    //     command::Command::ListLeaves => {
    //         let leaves = wallet.list_leaves().await?;
    //         println!("{}", serde_json::to_string_pretty(&leaves)?);
    //     }
    //     command::Command::SparkAddress => {
    //         let spark_address = wallet.get_spark_address().await?;
    //         println!("{}", spark_address.to_address_string()?);
    //     }
    //     command::Command::Transfer {
    //         amount_sat,
    //         receiver_address,
    //     } => {
    //         let result = wallet.transfer(amount_sat, &receiver_address).await?;
    //         println!("{}", serde_json::to_string_pretty(&result)?);
    //     }
    //     command::Command::ClaimPendingTransfers => {
    //         let transfers = wallet.claim_pending_transfers().await?;
    //         println!(
    //             "Claimed transfers: {}",
    //             serde_json::to_string_pretty(&transfers)?
    //         );
    //     }
    // }

    Ok(())
}
