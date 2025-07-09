use clap::Subcommand;
use spark_wallet::{SparkAddress, SparkWallet};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
pub enum TransferCommand {
    /// Claims all pending transfers
    ClaimPending,

    /// Lists all transfers
    List,

    /// Lists all pending transfers
    ListPending,

    /// Transfer funds to another wallet.
    Transfer {
        /// The amount to transfer in satoshis.
        amount_sat: u64,
        /// The receiver's Spark address.
        receiver_address: SparkAddress,
    },
}

pub async fn handle_command<S>(
    _config: &Config,
    wallet: &SparkWallet<S>,
    command: TransferCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        TransferCommand::ClaimPending => {
            let transfers = wallet.claim_pending_transfers().await?;
            println!(
                "Claimed transfers: {}",
                serde_json::to_string_pretty(&transfers)?
            );
        }
        TransferCommand::List => {
            let transfers = wallet.list_transfers().await?;
            println!("Transfers: {}", serde_json::to_string_pretty(&transfers)?);
        }
        TransferCommand::ListPending => {
            let transfers = wallet.list_pending_transfers().await?;
            println!(
                "Pending transfers: {}",
                serde_json::to_string_pretty(&transfers)?
            );
        }
        TransferCommand::Transfer {
            amount_sat,
            receiver_address,
        } => {
            let result = wallet.transfer(amount_sat, &receiver_address).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(())
}
