use clap::Parser;
use rustyline::{Editor, history::DefaultHistory};
use spark_wallet::SparkWallet;

use crate::{
    CliHelper,
    command::{
        deposit::DepositCommand, leaves::LeavesCommand, lightning::LightningCommand,
        transfer::TransferCommand,
    },
    config::Config,
};

pub mod deposit;
pub mod leaves;
pub mod lightning;
pub mod transfer;

#[derive(Clone, Debug, Parser)]
pub enum Command {
    /// Display the wallet's available balance.
    Balance,
    /// Display the wallet's info.
    Info,
    /// Display the wallet's Spark address.
    SparkAddress,
    /// Sync the wallet with the latest state.
    Sync,
    /// Deposit commands.
    #[command(subcommand)]
    Deposit(DepositCommand),
    /// Leaves commands.
    #[command(subcommand)]
    Leaves(LeavesCommand),
    /// Lightning commands.
    #[command(subcommand)]
    Lightning(LightningCommand),
    /// Transfer commands.
    #[command(subcommand)]
    Transfer(TransferCommand),
    /// REGTEST ONLY: Request funds from the faucet.
    RequestRegtestFunds {
        /// Amount in sats to request.
        amount_sats: u64,
        /// Address to receive the funds.
        address: String,
    },
}

pub(crate) async fn handle_command<S>(
    _rl: &mut Editor<CliHelper, DefaultHistory>,
    config: &Config,
    wallet: &SparkWallet<S>,
    command: Command,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        Command::Balance => {
            let balance = wallet.get_balance().await?;
            println!("Balance: {balance} sats")
        }
        Command::Deposit(deposit_command) => {
            deposit::handle_command(config, wallet, deposit_command).await?
        }
        Command::Info => {
            let info = wallet.get_info().await?;
            println!("{}", serde_json::to_string_pretty(&info)?)
        }
        Command::Leaves(leaves_command) => {
            leaves::handle_command(config, wallet, leaves_command).await?
        }
        Command::Lightning(lightning_command) => {
            lightning::handle_command(config, wallet, lightning_command).await?
        }
        Command::SparkAddress => {
            let spark_address = wallet.get_spark_address().await?;
            println!("{}", spark_address.to_address_string()?)
        }
        Command::Sync => {
            wallet.sync().await?;
            println!("Wallet synced successfully.")
        }
        Command::Transfer(transfer_command) => {
            transfer::handle_command(config, wallet, transfer_command).await?
        }
        Command::RequestRegtestFunds {
            amount_sats,
            address,
        } => {
            let Some(faucet_username) = &config.faucet_username else {
                return Err("Faucet username is required for regtest network. Please set SPARK_FAUCET_USERNAME environment variable".into());
            };
            let Some(faucet_password) = &config.faucet_password else {
                return Err("Faucet password is required for regtest network. Please set SPARK_FAUCET_PASSWORD environment variable".into());
            };
            if amount_sats < 1000 || amount_sats > 50_000 {
                return Err("Amount to request must be between 1000 and 50000 sats".into());
            }
            let txid = wallet
                .request_regtest_funds(
                    amount_sats,
                    address.parse()?,
                    faucet_username,
                    faucet_password,
                )
                .await?;
            println!("Requested regtest funds. Transaction ID: {txid}");
        }
    }

    Ok(())
}
