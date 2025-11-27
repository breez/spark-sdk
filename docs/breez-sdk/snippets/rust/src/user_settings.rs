use anyhow::Result;
use breez_sdk_spark::*;
use tracing::info;

pub(crate) async fn get_user_settings(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: get-user-settings
    let userSettings = sdk.get_user_settings().await?;
    info!("User settings: {:?}", userSettings);
    // ANCHOR_END: get-user-settings
    Ok(())
}

pub(crate) async fn update_user_settings(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: update-user-settings
    let sparkPrivateModeEnabled = true;
    sdk.update_user_settings(UpdateUserSettingsRequest {
        spark_private_mode_enabled: Some(sparkPrivateModeEnabled),
    }).await?;
    // ANCHOR_END: update-user-settings
    Ok(())
}