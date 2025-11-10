use anyhow::Result;
use breez_sdk_itest::ReinitializableSdkInstance;
use breez_sdk_spark::*;
use rstest::*;
use tempdir::TempDir;
use tracing::info;

// ---------------------
// Fixtures
// ---------------------

#[fixture]
fn persistent_sdk_private() -> ReinitializableSdkInstance {
    let mut cfg = default_config(Network::Regtest);
    cfg.private_enabled_default = true;
    ReinitializableSdkInstance::new(cfg, TempDir::new("breez-sdk-persistent-private").unwrap())
        .unwrap()
}

#[fixture]
fn persistent_sdk_non_private() -> ReinitializableSdkInstance {
    let mut cfg = default_config(Network::Regtest);
    cfg.private_enabled_default = false;
    ReinitializableSdkInstance::new(
        cfg,
        TempDir::new("breez-sdk-persistent-non-private").unwrap(),
    )
    .unwrap()
}

/// Test 1: Private mode user setting
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_spark_private_mode_user_setting(
    persistent_sdk_private: ReinitializableSdkInstance,
    persistent_sdk_non_private: ReinitializableSdkInstance,
) -> Result<()> {
    info!("=== Starting test_01_spark_private_mode_user_setting ===");

    let default_private = persistent_sdk_private.build_sdk().await?;
    let default_non_private = persistent_sdk_non_private.build_sdk().await?;

    // Check initial settings
    let initial_private_settings = default_private.sdk.get_user_settings().await?;
    assert!(initial_private_settings.spark_private_mode_enabled);
    let initial_non_private_settings = default_non_private.sdk.get_user_settings().await?;
    assert!(!initial_non_private_settings.spark_private_mode_enabled);

    // Update user settings
    default_private
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            enable_spark_private_mode: Some(false),
        })
        .await?;
    default_non_private
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            enable_spark_private_mode: Some(true),
        })
        .await?;

    // Verify settings were updated
    let updated_private_settings = default_private.sdk.get_user_settings().await?;
    assert!(!updated_private_settings.spark_private_mode_enabled);
    let updated_non_private_settings = default_non_private.sdk.get_user_settings().await?;
    assert!(updated_non_private_settings.spark_private_mode_enabled);

    // Re-initialize the SDKs and check that the user settings persist after reinitialization
    info!("=== Testing user settings persistence across SDK reinitialization ===");

    // Drop the current SDK instances
    drop(default_private);
    drop(default_non_private);

    // Reinitialize SDKs with the same configuration
    let reinitialized_private = persistent_sdk_private.build_sdk().await?;
    let reinitialized_non_private = persistent_sdk_non_private.build_sdk().await?;

    // Verify that user settings persist after reinitialization
    let reinitialized_private_settings = reinitialized_private.sdk.get_user_settings().await?;
    assert!(!reinitialized_private_settings.spark_private_mode_enabled);
    let reinitialized_non_private_settings =
        reinitialized_non_private.sdk.get_user_settings().await?;
    assert!(reinitialized_non_private_settings.spark_private_mode_enabled);

    info!("=== Test test_01_spark_private_mode_user_setting PASSED ===");
    Ok(())
}
