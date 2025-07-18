use bitcoin::{Address, address::NetworkUnchecked};
use clap::Subcommand;
use spark_wallet::SparkWallet;

use crate::config::Config;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum ExitSpeed {
    Fast,
    Medium,
    Slow,
}

impl From<ExitSpeed> for spark_wallet::ExitSpeed {
    fn from(speed: ExitSpeed) -> Self {
        match speed {
            ExitSpeed::Fast => spark_wallet::ExitSpeed::Fast,
            ExitSpeed::Medium => spark_wallet::ExitSpeed::Medium,
            ExitSpeed::Slow => spark_wallet::ExitSpeed::Slow,
        }
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum WithdrawCommand {
    /// Fetch the current coop exit fee quote.
    FetchFeeQuote {
        withdrawal_address: String,
        amount_sats: Option<u64>,
    },
    /// Perform a coop exit.
    CoopExit {
        withdrawal_address: String,
        exit_speed: ExitSpeed,
        amount_sats: Option<u64>,
    },
}

pub async fn handle_command<S>(
    _config: &Config,
    wallet: &SparkWallet<S>,
    command: WithdrawCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        WithdrawCommand::FetchFeeQuote {
            withdrawal_address,
            amount_sats,
        } => {
            let withdrawal_address = withdrawal_address.parse()?;
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(withdrawal_address, amount_sats)
                .await?;
            println!("{}", serde_json::to_string_pretty(&fee_quote)?);
        }
        WithdrawCommand::CoopExit {
            withdrawal_address,
            exit_speed,
            amount_sats,
        } => {
            let withdrawal_address: Address<NetworkUnchecked> = withdrawal_address.parse()?;
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(withdrawal_address.clone(), amount_sats)
                .await?;

            let result = wallet
                .withdraw(
                    withdrawal_address,
                    amount_sats,
                    exit_speed.into(),
                    fee_quote,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(())
}
