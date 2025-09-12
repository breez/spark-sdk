use clap::Subcommand;
use spark_wallet::{PagingFilter, SparkAddress, SparkWallet};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum TransferCommand {
    /// Claims all pending transfers
    ClaimPending,

    /// Lists all transfers
    List {
        /// The maximum number of transfers to return.
        #[clap(short, long)]
        limit: Option<u64>,
        /// The offset to start listing transfers from.
        #[clap(short, long)]
        offset: Option<u64>,
    },

    /// Lists all pending transfers
    ListPending {
        /// The maximum number of transfers to return.
        #[clap(short, long)]
        limit: Option<u64>,
        /// The offset to start listing transfers from.
        #[clap(short, long)]
        offset: Option<u64>,
    },

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
        TransferCommand::List { limit, offset } => {
            let paging = if limit.is_some() || offset.is_some() {
                Some(PagingFilter::new(offset, limit, None))
            } else {
                None
            };

            let transfers = wallet.list_transfers(paging, None).await?;
            println!("Transfers: {}", serde_json::to_string_pretty(&transfers)?);
        }
        TransferCommand::ListPending { limit, offset } => {
            let paging = if limit.is_some() || offset.is_some() {
                Some(PagingFilter::new(offset, limit, None))
            } else {
                None
            };

            let transfers = wallet.list_pending_transfers(paging).await?;
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
