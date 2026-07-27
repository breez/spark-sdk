use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn get_user_settings(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: get-user-settings
    let user_settings = sdk.get_user_settings().await?;
    info!("User settings: {:?}", user_settings);
    // ANCHOR_END: get-user-settings
    Ok(())
}

pub(crate) async fn update_user_settings(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: update-user-settings
    // Fields left as None are not changed. Settings that take an enum value
    // accept Set to assign one, or Unset to clear it.
    sdk.update_user_settings(UpdateUserSettingsRequest {
        spark_private_mode_enabled: Some(true),
        stable_balance_active_label: None,
        spark_master_identity_public_key: Some(SparkMasterIdentityPublicKey::Set {
            public_key: "<hex encoded public key>".to_string(),
        }),
    })
    .await?;
    // ANCHOR_END: update-user-settings
    Ok(())
}

pub(crate) async fn activate_stable_balance(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: activate-stable-balance
    sdk.update_user_settings(UpdateUserSettingsRequest {
        spark_private_mode_enabled: None,
        stable_balance_active_label: Some(StableBalanceActiveLabel::Set {
            label: "USDB".to_string(),
        }),
        spark_master_identity_public_key: None,
    })
    .await?;
    // ANCHOR_END: activate-stable-balance
    Ok(())
}

pub(crate) async fn deactivate_stable_balance(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: deactivate-stable-balance
    sdk.update_user_settings(UpdateUserSettingsRequest {
        spark_private_mode_enabled: None,
        stable_balance_active_label: Some(StableBalanceActiveLabel::Unset),
        spark_master_identity_public_key: None,
    })
    .await?;
    // ANCHOR_END: deactivate-stable-balance
    Ok(())
}
