use bitcoin::secp256k1::{PublicKey, ecdsa::Signature};
use clap::Parser;
use rustyline::{Editor, history::DefaultHistory};
use spark_wallet::{Network, SparkWallet};

use crate::{
    CliHelper,
    command::{
        deposit::DepositCommand, htlc::HtlcCommand, invoices::InvoicesCommand,
        leaves::LeavesCommand, lightning::LightningCommand, tokens::TokensCommand,
        transfer::TransferCommand, wallet_settings::WalletSettingsCommand,
        withdraw::WithdrawCommand,
    },
    config::MempoolConfig,
};

pub mod deposit;
pub mod htlc;
pub mod invoices;
pub mod leaves;
pub mod lightning;
pub mod tokens;
pub mod transfer;
pub mod wallet_settings;
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
    /// Invoices commands.
    #[command(subcommand)]
    Invoices(InvoicesCommand),
    /// Wallet settings commands.
    #[command(subcommand)]
    WalletSettings(WalletSettingsCommand),
    /// HTLC commands.
    #[command(subcommand)]
    Htlc(HtlcCommand),
}

pub(crate) async fn handle_command(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    network: Network,
    mempool_config: &MempoolConfig,
    wallet: &SparkWallet,
    command: Command,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Balance => {
            let balance = wallet.get_balance().await?;
            println!("Balance: {balance} sats");
        }
        Command::Deposit(deposit_command) => {
            deposit::handle_command(mempool_config, wallet, deposit_command).await?
        }
        Command::Info => {
            let info = wallet.get_info();
            println!("{}", serde_json::to_string_pretty(&info)?)
        }
        Command::Leaves(leaves_command) => leaves::handle_command(wallet, leaves_command).await?,
        Command::Lightning(lightning_command) => {
            lightning::handle_command(wallet, lightning_command).await?
        }
        Command::Tokens(tokens_command) => tokens::handle_command(wallet, tokens_command).await?,
        Command::Sign => {
            let message = rl.readline("Enter message to sign: ")?;
            let signature = wallet.sign_message(&message).await?;
            let signature = hex::encode(signature.serialize_der());
            println!("Signature: {signature}");
        }
        Command::SparkAddress => {
            let spark_address = wallet.get_spark_address()?.to_address_string()?;
            println!("{spark_address}")
        }
        Command::Sync => {
            wallet.sync().await?;
            println!("Wallet synced successfully.")
        }
        Command::Transfer(transfer_command) => {
            transfer::handle_command(wallet, transfer_command).await?
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
            withdraw::handle_command(network, wallet, withdraw_command).await?
        }
        Command::Invoices(invoices_command) => {
            invoices::handle_command(wallet, invoices_command).await?
        }
        Command::WalletSettings(wallet_settings_command) => {
            wallet_settings::handle_command(wallet, wallet_settings_command).await?
        }
        Command::Htlc(htlc_command) => htlc::handle_command(wallet, htlc_command).await?,
    }

    Ok(())
}
