use anyhow::Result;
use breez_sdk_itest::ReinitializableSdkInstance;
use breez_sdk_itest::fixtures::{BEAN_REGTEST_TOKEN_ID, SHELL_REGTEST_TOKEN_ID};
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
            stable_balance_active_ticker: None,
            spark_private_mode_enabled: Some(false),
        })
        .await?;
    default_non_private
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            stable_balance_active_ticker: None,
            spark_private_mode_enabled: Some(true),
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

// ---------------------
// Stable Balance Fixtures
// ---------------------

#[fixture]
fn persistent_sdk_stable_balance() -> ReinitializableSdkInstance {
    let mut cfg = default_config(Network::Regtest);
    cfg.stable_balance_config = Some(StableBalanceConfig {
        tokens: vec![
            StableBalanceToken {
                ticker: "SHELL".to_string(),
                token_identifier: SHELL_REGTEST_TOKEN_ID.to_string(),
            },
            StableBalanceToken {
                ticker: "BEAN".to_string(),
                token_identifier: BEAN_REGTEST_TOKEN_ID.to_string(),
            },
        ],
        default_active_ticker: Some("SHELL".to_string()),
        threshold_sats: Some(1000),
        max_slippage_bps: Some(500),
    });
    ReinitializableSdkInstance::new(
        cfg,
        TempDir::new("breez-sdk-persistent-stable-balance").unwrap(),
    )
    .unwrap()
}

/// Test 2: Stable balance active ticker user setting
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_stable_balance_user_setting(
    persistent_sdk_stable_balance: ReinitializableSdkInstance,
) -> Result<()> {
    info!("=== Starting test_02_stable_balance_user_setting ===");

    let sdk_instance = persistent_sdk_stable_balance.build_sdk().await?;

    // Check initial setting — default is SHELL
    let settings = sdk_instance.sdk.get_user_settings().await?;
    assert_eq!(
        settings.stable_balance_active_ticker.as_deref(),
        Some("SHELL")
    );

    // Unset the active ticker
    sdk_instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_ticker: Some(StableBalanceActiveTicker::Unset),
        })
        .await?;
    let settings = sdk_instance.sdk.get_user_settings().await?;
    assert_eq!(settings.stable_balance_active_ticker, None);

    // Set to BEAN
    sdk_instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_ticker: Some(StableBalanceActiveTicker::Set {
                ticker: "BEAN".to_string(),
            }),
        })
        .await?;
    let settings = sdk_instance.sdk.get_user_settings().await?;
    assert_eq!(
        settings.stable_balance_active_ticker.as_deref(),
        Some("BEAN")
    );

    // Verify persistence across SDK reinitialization
    info!("=== Testing stable balance setting persistence across SDK reinitialization ===");
    drop(sdk_instance);

    let reinitialized = persistent_sdk_stable_balance.build_sdk().await?;
    let settings = reinitialized.sdk.get_user_settings().await?;
    assert_eq!(
        settings.stable_balance_active_ticker.as_deref(),
        Some("BEAN")
    );

    info!("=== Test test_02_stable_balance_user_setting PASSED ===");
    Ok(())
}
