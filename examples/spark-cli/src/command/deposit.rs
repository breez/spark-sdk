use bitcoin::{Transaction, consensus::encode::deserialize_hex};
use clap::Subcommand;
use spark_wallet::{PagingFilter, SparkWallet};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
pub enum DepositCommand {
    /// Claim a deposit after it has been confirmed onchain.
    Claim {
        /// The transaction ID of the deposit transaction.
        txid: String,
    },
    /// Generate a new onchain deposit address.
    NewAddress,
    /// List unused deposit addresses.
    ListUnusedAddresses {
        /// The maximum number of addresses to return.
        #[clap(short, long)]
        limit: Option<u64>,
        /// The offset to start listing addresses from.
        #[clap(short, long)]
        offset: Option<u64>,
    },
}

pub async fn handle_command<S>(
    config: &Config,
    wallet: &SparkWallet<S>,
    command: DepositCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        DepositCommand::NewAddress => {
            let address = wallet.generate_deposit_address(false).await?;
            println!("{address}");
        }
        DepositCommand::Claim { txid } => {
            let tx = get_transaction(config, txid.clone()).await?;
            // TODO: Look for correct output index
            for (vout, _) in tx.output.iter().enumerate() {
                println!("Checking output {vout} for txid: {txid}");
                if let Ok(leaves) = wallet.claim_deposit(tx.clone(), vout as u32).await {
                    println!(
                        "Claimed deposit: {}",
                        serde_json::to_string_pretty(&leaves)?
                    );
                    return Ok(());
                }
            }

            println!("Could not claim deposit for txid: {txid} - no matching output found.",);
        }
        DepositCommand::ListUnusedAddresses { limit, offset } => {
            let paging = if limit.is_some() || offset.is_some() {
                Some(PagingFilter::new(offset, limit))
            } else {
                None
            };
            let addresses = wallet.list_unused_deposit_addresses(paging).await?;
            println!("{}", serde_json::to_string_pretty(&addresses)?);
        }
    }

    Ok(())
}

async fn get_transaction(
    config: &Config,
    txid: String,
) -> Result<Transaction, Box<dyn std::error::Error>> {
    let url = format!("{}/tx/{}/hex", config.mempool_url, txid);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .basic_auth(
            config.mempool_username.clone(),
            Some(config.mempool_password.clone()),
        )
        .send()
        .await?;
    let hex = response.text().await?;
    let tx = deserialize_hex(&hex)?;
    Ok(tx)
}
