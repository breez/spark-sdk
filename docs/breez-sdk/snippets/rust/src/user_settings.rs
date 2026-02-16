use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn get_user_settings(client: &BreezClient) -> Result<()> {
    // ANCHOR: get-user-settings
    let user_settings = client.settings().get().await?;
    info!("User settings: {:?}", user_settings);
    // ANCHOR_END: get-user-settings
    Ok(())
}

pub(crate) async fn update_user_settings(client: &BreezClient) -> Result<()> {
    // ANCHOR: update-user-settings
    let spark_private_mode_enabled = true;
    client.settings().update(UpdateUserSettingsRequest {
        spark_private_mode_enabled: Some(spark_private_mode_enabled),
    })
    .await?;
    // ANCHOR_END: update-user-settings
    Ok(())
}
