use std::{
    str::FromStr,
    time::{Duration, SystemTime},
};

use clap::Subcommand;
use spark_wallet::{PublicKey, SparkWallet};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum InvoicesCommand {
    /// Creates a spark invoice
    Create {
        #[clap(short, long)]
        /// The amount for the payment in base units.
        amount: Option<u128>,
        #[clap(short, long)]
        /// The token identifier for the token payment. Absence indicates a sats payment.
        token_identifier: Option<String>,
        #[clap(short, long)]
        /// The expiry time for the invoice in seconds from now.
        expiry_secs: Option<u64>,
        #[clap(short, long)]
        /// The description for the invoice.
        description: Option<String>,
        #[clap(short, long)]
        /// The sender public key for the invoice.
        sender_public_key: Option<String>,
    },

    /// Fulfills a spark invoice
    Fulfill {
        #[clap(short, long)]
        /// The invoice to fulfill.
        invoice: String,
        #[clap(short, long)]
        /// The amount to pay in base units. Must be provided if the invoice doesn't include an amount.
        amount: Option<u128>,
    },

    /// Queries a list of spark invoices
    Query {
        #[clap(short, long)]
        /// The list of invoices to query.
        invoices: Vec<String>,
    },
}

pub async fn handle_command(
    _config: &Config,
    wallet: &SparkWallet,
    command: InvoicesCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        InvoicesCommand::Create {
            amount,
            token_identifier,
            expiry_secs,
            description,
            sender_public_key,
        } => {
            let invoice = wallet.create_spark_invoice(
                amount,
                token_identifier,
                expiry_secs.map(|secs| SystemTime::now() + Duration::from_secs(secs)),
                description,
                sender_public_key
                    .map(|key| PublicKey::from_str(&key))
                    .transpose()?,
            )?;
            println!("Invoice: {}", invoice);
        }
        InvoicesCommand::Fulfill { invoice, amount } => {
            let result = wallet.fulfill_spark_invoice(&invoice, amount, None).await?;
            println!(
                "Fulfillment result: {}",
                serde_json::to_string_pretty(&result)?
            );
        }
        InvoicesCommand::Query { invoices } => {
            let results = wallet.query_spark_invoices(invoices).await?;
            println!("Query results: {}", serde_json::to_string_pretty(&results)?);
        }
    }
    Ok(())
}
