use clap::Subcommand;
use spark_wallet::SparkWallet;

#[derive(Debug, Subcommand)]
pub enum WalletSettingsCommand {
    /// Query the wallet settings.
    Query,
    /// Update the wallet settings.
    Update {
        #[clap(short, long)]
        /// Whether private mode is enabled.
        private_enabled: bool,
    },
}

pub async fn handle_command(
    wallet: &SparkWallet,
    command: WalletSettingsCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        WalletSettingsCommand::Query => {
            let settings = wallet.query_wallet_settings().await?;
            println!("{}", serde_json::to_string_pretty(&settings)?);
        }
        WalletSettingsCommand::Update { private_enabled } => {
            wallet.update_wallet_settings(private_enabled).await?;
            println!("Wallet settings updated successfully.");
        }
    }
    Ok(())
}
