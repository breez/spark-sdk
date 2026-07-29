use clap::Subcommand;
use spark_wallet::{MasterIdentityPublicKeyUpdate, PublicKey, SparkWallet};

#[derive(Debug, Subcommand)]
pub enum WalletSettingsCommand {
    /// Query the wallet settings.
    Query,
    /// Update the wallet settings.
    Update {
        #[clap(short, long)]
        /// Whether private mode is enabled.
        private_enabled: Option<bool>,

        /// Hex encoded public key to designate as the wallet's master identity.
        #[clap(short, long, conflicts_with = "clear_master_key")]
        master_key: Option<PublicKey>,

        /// Remove the wallet's designated master identity.
        #[clap(long, action = clap::ArgAction::SetTrue)]
        clear_master_key: bool,
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
        WalletSettingsCommand::Update {
            private_enabled,
            master_key,
            clear_master_key,
        } => {
            let master_identity_public_key = match (master_key, clear_master_key) {
                (Some(key), _) => Some(MasterIdentityPublicKeyUpdate::Set(key)),
                (None, true) => Some(MasterIdentityPublicKeyUpdate::Clear),
                (None, false) => None,
            };
            wallet
                .update_wallet_settings(private_enabled, master_identity_public_key)
                .await?;
            println!("Wallet settings updated successfully.");
        }
    }
    Ok(())
}
