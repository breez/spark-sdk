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
        amount_sat: Option<u64>,
    },
    /// Perform a coop exit.
    CoopExit {
        withdrawal_address: String,
        exit_speed: ExitSpeed,
        amount_sat: Option<u64>,
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
            amount_sat,
            withdrawal_address,
        } => {
            let withdrawal_address = withdrawal_address.parse()?;
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(withdrawal_address, amount_sat)
                .await?;
            println!("{}", serde_json::to_string_pretty(&fee_quote)?);
        }
        WithdrawCommand::CoopExit {
            withdrawal_address,
            exit_speed,
            amount_sat,
        } => {
            let withdrawal_address = withdrawal_address.parse()?;
            let result = wallet
                .withdraw(withdrawal_address, exit_speed.into(), amount_sat, None)
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(())
}
