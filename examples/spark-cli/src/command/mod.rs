use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use clap::Parser;
use rustyline::{Editor, history::DefaultHistory};
use spark_wallet::SparkWallet;

use crate::{
    CliHelper,
    command::{
        deposit::DepositCommand, leaves::LeavesCommand, lightning::LightningCommand,
        tokens::TokensCommand, transfer::TransferCommand, withdraw::WithdrawCommand,
    },
    config::Config,
};

pub mod deposit;
pub mod leaves;
pub mod lightning;
pub mod tokens;
pub mod transfer;
pub mod withdraw;

#[derive(Debug, Parser)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Display the wallet's available balance.
    Balance,
    /// Display the wallet's info.
    Info,
    /// Display the wallet's Spark address.
    SparkAddress,
    /// Sync the wallet with the latest state.
    Sync,
    /// Sign a message.
    Sign,
    /// Verify a message.
    Verify,
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
    /// Withdraw commands.
    #[command(subcommand)]
    Withdraw(WithdrawCommand),
    /// Token commands.
    #[command(subcommand)]
    Tokens(TokensCommand),
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
            println!("Balance: {balance} sats");
        }
        Command::Deposit(deposit_command) => {
            deposit::handle_command(config, wallet, deposit_command).await?
        }
        Command::Info => {
            let info = wallet.get_info();
            println!("{}", serde_json::to_string_pretty(&info)?)
        }
        Command::Leaves(leaves_command) => {
            leaves::handle_command(config, wallet, leaves_command).await?
        }
        Command::Lightning(lightning_command) => {
            lightning::handle_command(config, wallet, lightning_command).await?
        }
        Command::Tokens(tokens_command) => {
            tokens::handle_command(config, wallet, tokens_command).await?
        }
        Command::Sign => {
            let message = rl.readline("Enter message to sign: ")?;
            let signature = wallet.sign_message(&message).await?;
            let signature = hex::encode(signature.serialize_der());
            println!("Signature: {signature}");
        }
        Command::SparkAddress => {
            let spark_address = wallet.get_spark_address()?;
            println!("{spark_address}")
        }
        Command::Sync => {
            wallet.sync().await?;
            println!("Wallet synced successfully.")
        }
        Command::Transfer(transfer_command) => {
            transfer::handle_command(config, wallet, transfer_command).await?
        }
        Command::Verify => {
            let message = rl.readline("Enter message to verify: ")?;
            let signature = rl.readline("Enter signature to verify: ")?;
            let signature = Signature::from_der(&hex::decode(&signature)?)?;
            let public_key = rl.readline("Enter signer public key: ")?;
            let public_key = PublicKey::from_slice(&hex::decode(&public_key)?)?;
            wallet
                .verify_message(&message, &signature, &public_key)
                .await?;
            println!("Signature verified successfully.");
        }
        Command::Withdraw(withdraw_command) => {
            withdraw::handle_command(config, wallet, withdraw_command).await?
        }
    }

    Ok(())
}
