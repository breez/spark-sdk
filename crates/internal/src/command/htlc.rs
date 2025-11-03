use std::{str::FromStr, time::Duration};

use clap::Subcommand;
use rand::RngCore;
use spark_wallet::{PagingFilter, Preimage, SparkAddress, SparkWallet};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
pub enum HtlcCommand {
    /// List HTLCs.
    List {
        /// The maximum number of HTLCs to return.
        #[clap(short, long)]
        limit: Option<u64>,
        /// The offset to start listing HTLCs from.
        #[clap(short, long)]
        offset: Option<u64>,
    },
    /// Create an HTLC.
    Create {
        /// The amount of sats to send.
        #[clap(short, long)]
        amount_sat: u64,
        /// The receiver address.
        #[clap(short, long)]
        receiver_address: String,
        /// The expiry duration in seconds.
        #[clap(short, long)]
        expiry_secs: u64,
    },
    /// Claim an HTLC.
    Claim {
        /// The preimage.
        #[clap(short, long)]
        preimage: String,
    },
}

pub async fn handle_command(
    _config: &Config,
    wallet: &SparkWallet,
    command: HtlcCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        HtlcCommand::List { limit, offset } => {
            let htlcs = wallet
                .list_claimable_htlcs(Some(PagingFilter::new(limit, offset, None)))
                .await?;
            println!("HTLCs: {}", serde_json::to_string_pretty(&htlcs)?);
        }
        HtlcCommand::Create {
            amount_sat,
            receiver_address,
            expiry_secs,
        } => {
            // Parse receiver address
            let receiver_addr = SparkAddress::from_str(&receiver_address)?;

            // Generate random preimage
            let mut preimage_bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut preimage_bytes);
            let preimage = Preimage::try_from(preimage_bytes.to_vec())?;

            // Convert expiry to Duration
            let expiry_duration = Duration::from_secs(expiry_secs);

            println!(
                "Creating HTLC for {} sats to {} with expiry {} and preimage {}",
                amount_sat,
                receiver_address,
                expiry_duration.as_secs(),
                preimage.encode_hex()
            );

            let transfer = wallet
                .create_htlc(amount_sat, &receiver_addr, &preimage, expiry_duration)
                .await?;
            println!("Transfer: {}", serde_json::to_string_pretty(&transfer)?);
        }
        HtlcCommand::Claim { preimage } => {
            let preimage = Preimage::from_hex(&preimage)?;
            let transfer = wallet.claim_htlc(&preimage).await?;
            println!("Transfer: {}", serde_json::to_string_pretty(&transfer)?);
        }
    }
    Ok(())
}
