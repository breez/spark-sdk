use std::collections::HashMap;
use std::str::FromStr;

use bitcoin::{
    Transaction, Txid,
    consensus::encode::{deserialize_hex, serialize_hex},
};
use clap::Subcommand;
use platform_utils::{
    ContentType, DefaultHttpClient, HttpClient, add_basic_auth_header, add_content_type_header,
};
use spark_wallet::{Fee, PagingFilter, SparkWallet};

use crate::config::MempoolConfig;

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
        fee_sat: Option<u64>,
        /// The fee rate to pay for the refund transaction.
        sat_per_vbyte: Option<u64>,
        /// The output index of the static deposit transaction to refund.
        output_index: Option<u32>,
    },
}

pub async fn handle_command(
    mempool_config: &MempoolConfig,
    wallet: &SparkWallet,
    command: DepositCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        DepositCommand::NewAddress { is_static } => {
            let address = wallet.generate_deposit_address(is_static).await?;
            println!("{address}");
        }
        DepositCommand::FetchStaticClaimQuote { txid, output_index } => {
            let tx = get_transaction(mempool_config, txid.clone()).await?;
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
            let tx = get_transaction(mempool_config, txid.clone()).await?;
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
                Some(PagingFilter::new(offset, limit, None))
            } else {
                None
            };
            let addresses = wallet.list_static_deposit_addresses(paging).await?.items;
            println!("{}", serde_json::to_string_pretty(&addresses)?);
        }
        DepositCommand::ListUnusedAddresses { limit, offset } => {
            let paging = if limit.is_some() || offset.is_some() {
                Some(PagingFilter::new(offset, limit, None))
            } else {
                None
            };
            let addresses = wallet.list_unused_deposit_addresses(paging).await?;
            println!("{}", serde_json::to_string_pretty(&addresses.items)?);
        }
        DepositCommand::Refund {
            txid,
            refund_address,
            fee_sat,
            sat_per_vbyte,
            output_index,
        } => {
            let fee = match (fee_sat, sat_per_vbyte) {
                (Some(_), Some(_)) => {
                    println!("Cannot specify both fee_sat and sat_per_vbyte");
                    return Ok(());
                }
                (Some(fee_sat), None) => Fee::Fixed { amount: fee_sat },
                (None, Some(sat_per_vbyte)) => Fee::Rate { sat_per_vbyte },
                (None, None) => {
                    println!("Must specify either fee_sat or sat_per_vbyte");
                    return Ok(());
                }
            };

            let tx = get_transaction(mempool_config, txid.clone()).await?;
            let refund_tx = wallet
                .refund_static_deposit(tx, output_index, &refund_address, fee)
                .await?;
            let txid = broadcast_transaction(mempool_config, refund_tx).await?;
            println!("Refund txid: {txid}");
        }
    }

    Ok(())
}

async fn get_transaction(
    mempool_config: &MempoolConfig,
    txid: String,
) -> Result<Transaction, Box<dyn std::error::Error>> {
    let url = format!("{}/tx/{}/hex", mempool_config.url, txid);

    let mut headers = HashMap::new();
    if let (Some(username), Some(password)) = (&mempool_config.username, &mempool_config.password) {
        add_basic_auth_header(&mut headers, username, password);
    }

    let http_client = DefaultHttpClient::default();
    let response = http_client
        .get(url, Some(headers))
        .await
        .map_err(|e| format!("HTTP request failed: {e:?}"))?;

    let tx = deserialize_hex(&response.body)?;
    Ok(tx)
}

async fn broadcast_transaction(
    mempool_config: &MempoolConfig,
    tx: Transaction,
) -> Result<Txid, Box<dyn std::error::Error>> {
    let tx_hex = serialize_hex(&tx);
    let url = format!("{}/tx", mempool_config.url);

    let mut headers = HashMap::new();
    if let (Some(username), Some(password)) = (&mempool_config.username, &mempool_config.password) {
        add_basic_auth_header(&mut headers, username, password);
    }
    add_content_type_header(&mut headers, ContentType::TextPlain);

    let http_client = DefaultHttpClient::default();
    let response = http_client
        .post(url, Some(headers), Some(tx_hex.clone()))
        .await
        .map_err(|e| format!("HTTP request failed: {e:?}"))?;

    let txid = Txid::from_str(&response.body).map_err(|_| {
        println!("Refund tx hex: {tx_hex}");
        format!("Failed to parse txid from response: {}", response.body)
    })?;
    Ok(txid)
}
