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
}

pub(crate) async fn handle_command<S>(
    rl: &mut Editor<CliHelper, DefaultHistory>,
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
            deposit::handle_command(rl, &config, &wallet, deposit_command).await?
        }
        Command::Info => {
            let info = wallet.get_info().await?;
            println!("{}", serde_json::to_string_pretty(&info)?)
        }
        Command::Leaves(leaves_command) => {
            leaves::handle_command(&config, &wallet, leaves_command).await?
        }
        Command::Lightning(lightning_command) => {
            lightning::handle_command(&config, &wallet, lightning_command).await?
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
            transfer::handle_command(&config, &wallet, transfer_command).await?
        }
    }

    Ok(())
}
