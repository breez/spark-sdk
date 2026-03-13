use breez_sdk_spark::{BreezSdk, StableBalanceActiveTicker, UpdateUserSettingsRequest};
use clap::Subcommand;

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum StableBalanceCommand {
    /// Get the stable balance active ticker
    Get,
    /// Set the stable balance active ticker
    Set {
        /// The ticker to activate (e.g. "USDB")
        ticker: String,
    },
    /// Unset stable balance
    Unset,
}

pub async fn handle_command(
    sdk: &BreezSdk,
    command: StableBalanceCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        StableBalanceCommand::Get => {
            let settings = sdk.get_user_settings().await?;
            print_value(&settings.stable_balance_active_ticker)?;
            Ok(true)
        }
        StableBalanceCommand::Set { ticker } => {
            sdk.update_user_settings(UpdateUserSettingsRequest {
                spark_private_mode_enabled: None,
                stable_balance_active_ticker: Some(StableBalanceActiveTicker::Set { ticker }),
            })
            .await?;
            let settings = sdk.get_user_settings().await?;
            print_value(&settings)?;
            Ok(true)
        }
        StableBalanceCommand::Unset => {
            sdk.update_user_settings(UpdateUserSettingsRequest {
                spark_private_mode_enabled: None,
                stable_balance_active_ticker: Some(StableBalanceActiveTicker::Unset),
            })
            .await?;
            let settings = sdk.get_user_settings().await?;
            print_value(&settings)?;
            Ok(true)
        }
    }
}
