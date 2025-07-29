use std::str::FromStr;

use clap::Subcommand;
use spark_wallet::{PagingFilter, SparkAddress, SparkWallet, TransferTokenOutput};

use crate::config::Config;

/// A transfer output that can be parsed from a string in the format "token_id:amount:receiver_address"
#[derive(Debug, Clone)]
pub struct TransferTokenOutputArg {
    pub token_id: String,
    pub amount: u128,
    pub receiver_address: String,
}

impl FromStr for TransferTokenOutputArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return Err(format!(
                "Invalid format '{}'. Expected format: 'token_id:amount:receiver_address'",
                s
            ));
        }

        let token_id = parts[0].to_string();
        let amount = parts[1]
            .parse::<u128>()
            .map_err(|_| format!("Invalid amount '{}': must be a valid number", parts[1]))?;
        let receiver_address = parts[2].to_string();

        Ok(TransferTokenOutputArg {
            token_id,
            amount,
            receiver_address,
        })
    }
}

impl TryFrom<TransferTokenOutputArg> for TransferTokenOutput {
    type Error = Box<dyn std::error::Error>;

    fn try_from(arg: TransferTokenOutputArg) -> Result<Self, Self::Error> {
        Ok(TransferTokenOutput {
            token_id: arg.token_id,
            amount: arg.amount,
            receiver_address: SparkAddress::from_str(&arg.receiver_address)?,
        })
    }
}

#[derive(Debug, Subcommand)]
pub enum TokensCommand {
    /// Prints the L1 address of the token wallet.
    L1Address,
    /// Transfer tokens to another Spark user.
    Transfer {
        token_id: String,
        amount: u128,
        receiver_address: String,
    },
    /// Transfer tokens with multiple outputs.
    ///
    /// Example usage:
    /// tokens batch-transfer token_id1:100:address1 token_id2:200:address2
    BatchTransfer {
        outputs: Vec<TransferTokenOutputArg>,
    },
    /// List transfers
    ListTransactions {
        #[clap(short, long)]
        limit: Option<u64>,
        #[clap(short, long)]
        offset: Option<u64>,
    },
}

pub async fn handle_command<S>(
    _config: &Config,
    wallet: &SparkWallet<S>,
    command: TokensCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        TokensCommand::L1Address => {
            let l1_address = wallet.get_token_l1_address()?;
            println!("L1 address: {l1_address}");
            Ok(())
        }
        TokensCommand::Transfer {
            token_id,
            amount,
            receiver_address,
        } => {
            let output = TransferTokenOutput {
                token_id,
                amount,
                receiver_address: SparkAddress::from_str(&receiver_address)?,
            };
            let transfer_id = wallet.transfer_tokens(&output).await?;
            println!("Transaction ID: {transfer_id}");
            Ok(())
        }
        TokensCommand::BatchTransfer { outputs } => {
            if outputs.is_empty() {
                return Err("At least one output must be specified".into());
            }

            let outputs: Vec<TransferTokenOutput> = outputs
                .into_iter()
                .map(|o| o.try_into())
                .collect::<Result<Vec<_>, _>>()?;
            let transfer_id = wallet.batch_transfer_tokens(&outputs).await?;
            println!("Transaction ID: {transfer_id}");
            Ok(())
        }
        TokensCommand::ListTransactions { limit, offset } => {
            let paging = if limit.is_some() || offset.is_some() {
                Some(PagingFilter::new(offset, limit))
            } else {
                None
            };
            let transactions = wallet
                .query_token_transactions(None, None, None, None, None, paging)
                .await?;

            println!(
                "Transactions: {}",
                serde_json::to_string_pretty(&transactions)?
            );
            Ok(())
        }
    }
}
