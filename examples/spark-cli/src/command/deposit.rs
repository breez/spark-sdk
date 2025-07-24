use std::str::FromStr;

use bitcoin::{
    Transaction, Txid,
    consensus::encode::{deserialize_hex, serialize_hex},
};
use clap::Subcommand;
use reqwest::header::CONTENT_TYPE;
use spark_wallet::{PagingFilter, SparkWallet};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
pub enum DepositCommand {
    /// Claim a deposit after it has been confirmed onchain.
    Claim {
        /// The transaction ID of the deposit transaction.
        txid: String,
        /// The output index of the deposit transaction.
        output_index: Option<u32>,
        /// Whether the transaction is from a static deposit.
        #[arg(short = 's', long, default_value = "false")]
        is_static: bool,
    },
    /// Fetch a quote for the creditable amount.
    FetchStaticClaimQuote {
        /// The transaction ID of the static deposit transaction.
        txid: String,
        /// The output index of the static deposit transaction to claim.
        output_index: Option<u32>,
    },
    /// Generate a new onchain deposit address.
    NewAddress {
        /// Whether the address should be static (reusable).
        #[arg(short = 's', long, default_value = "false")]
        is_static: bool,
    },
    /// List static deposit addresses.
    ListStaticAddresses {
        /// The maximum number of addresses to return.
        #[clap(short, long)]
        limit: Option<u64>,
        /// The offset to start listing addresses from.
        #[clap(short, long)]
        offset: Option<u64>,
    },
    /// List unused deposit addresses.
    ListUnusedAddresses {
        /// The maximum number of addresses to return.
        #[clap(short, long)]
        limit: Option<u64>,
        /// The offset to start listing addresses from.
        #[clap(short, long)]
        offset: Option<u64>,
    },
    /// Refund a static deposit.
    Refund {
        /// The transaction ID of the static deposit transaction.
        txid: String,
        /// The address to send the refund to.
        refund_address: String,
        /// The fee to pay for the refund transaction.
        fee_sats: u64,
        /// The output index of the static deposit transaction to refund.
        output_index: Option<u32>,
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
        DepositCommand::NewAddress { is_static } => {
            let address = wallet.generate_deposit_address(is_static).await?;
            println!("{address}");
        }
        DepositCommand::FetchStaticClaimQuote { txid, output_index } => {
            let tx = get_transaction(config, txid.clone()).await?;
            let quote = wallet
                .fetch_static_deposit_claim_quote(tx, output_index)
                .await?;
            println!("{}", serde_json::to_string_pretty(&quote)?);
        }
        DepositCommand::Claim {
            txid,
            output_index,
            is_static,
        } => {
            let tx = get_transaction(config, txid.clone()).await?;
            if is_static {
                let quote = wallet
                    .fetch_static_deposit_claim_quote(tx.clone(), output_index)
                    .await?;
                let transfer = wallet.claim_static_deposit(quote).await?;
                println!("{}", serde_json::to_string_pretty(&transfer)?);
            } else if let Some(output_index) = output_index {
                let leaves = wallet.claim_deposit(tx, output_index).await?;
                println!("{}", serde_json::to_string_pretty(&leaves)?);
            } else {
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
        }
        DepositCommand::ListStaticAddresses { limit, offset } => {
            let paging = if limit.is_some() || offset.is_some() {
                Some(PagingFilter::new(offset, limit))
            } else {
                None
            };
            let addresses = wallet.list_static_deposit_addresses(paging).await?;
            println!("{}", serde_json::to_string_pretty(&addresses)?);
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
        DepositCommand::Refund {
            txid,
            refund_address,
            fee_sats,
            output_index,
        } => {
            let tx = get_transaction(config, txid.clone()).await?;
            let refund_tx = wallet
                .refund_static_deposit(tx, output_index, &refund_address, fee_sats)
                .await?;
            let txid = broadcast_transaction(config, refund_tx).await?;
            println!("Refund txid: {txid}");
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

async fn broadcast_transaction(
    config: &Config,
    tx: Transaction,
) -> Result<Txid, Box<dyn std::error::Error>> {
    let tx_hex = serialize_hex(&tx);
    let url = format!("{}/tx", config.mempool_url);
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .basic_auth(
            config.mempool_username.clone(),
            Some(config.mempool_password.clone()),
        )
        .header(CONTENT_TYPE, "text/plain")
        .body(tx_hex.clone())
        .send()
        .await?;
    let text = response.text().await?;
    let txid = Txid::from_str(&text).map_err(|_| {
        println!("Refund tx hex: {}", tx_hex);
        format!("Failed to parse txid from response: {text}")
    })?;
    Ok(txid)
}
