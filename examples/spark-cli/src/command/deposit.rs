use bitcoin::{Transaction, consensus::encode::deserialize_hex};
use clap::Subcommand;
use spark_wallet::SparkWallet;

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
    ListUnusedAddresses,
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
        DepositCommand::ListUnusedAddresses => {
            let addresses = wallet.list_unused_deposit_addresses().await?;
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
