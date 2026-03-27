use breez_sdk_spark::{BreezSdk, StableBalanceActiveLabel, UpdateUserSettingsRequest};
use clap::Subcommand;

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum StableBalanceCommand {
    /// Get the stable balance active label
    Get,
    /// Set the stable balance active label
    Set {
        /// The label to activate (e.g. "USDB")
        label: String,
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
            print_value(&settings.stable_balance_active_label)?;
            Ok(true)
        }
        StableBalanceCommand::Set { label } => {
            sdk.update_user_settings(UpdateUserSettingsRequest {
                spark_private_mode_enabled: None,
                stable_balance_active_label: Some(StableBalanceActiveLabel::Set { label }),
            })
            .await?;
            let settings = sdk.get_user_settings().await?;
            print_value(&settings)?;
            Ok(true)
        }
        StableBalanceCommand::Unset => {
            sdk.update_user_settings(UpdateUserSettingsRequest {
                spark_private_mode_enabled: None,
                stable_balance_active_label: Some(StableBalanceActiveLabel::Unset),
            })
            .await?;
            let settings = sdk.get_user_settings().await?;
            print_value(&settings)?;
            Ok(true)
        }
    }
}
