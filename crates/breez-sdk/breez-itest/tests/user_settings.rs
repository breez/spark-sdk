use anyhow::Result;
use breez_sdk_itest::ReinitializableSdkInstance;
use breez_sdk_itest::fixtures::{BEAN_REGTEST_TOKEN_ID, SHELL_REGTEST_TOKEN_ID};
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

// ---------------------
// Fixtures
// ---------------------

#[fixture]
fn persistent_sdk_private() -> ReinitializableSdkInstance {
    let mut cfg = default_config(Network::Regtest);
    cfg.private_enabled_default = true;
    ReinitializableSdkInstance::new(
        cfg,
        tempfile::Builder::new()
            .prefix("breez-sdk-persistent-private")
            .tempdir()
            .unwrap(),
    )
    .unwrap()
}

#[fixture]
fn persistent_sdk_non_private() -> ReinitializableSdkInstance {
    let mut cfg = default_config(Network::Regtest);
    cfg.private_enabled_default = false;
    ReinitializableSdkInstance::new(
        cfg,
        tempfile::Builder::new()
            .prefix("breez-sdk-persistent-non-private")
            .tempdir()
            .unwrap(),
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
            stable_balance_active_label: None,
            spark_master_identity_public_key: None,
            spark_private_mode_enabled: Some(false),
        })
        .await?;
    default_non_private
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            stable_balance_active_label: None,
            spark_master_identity_public_key: None,
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

/// Test 1b: Master identity public key user setting
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01b_spark_master_identity_public_key_user_setting(
    persistent_sdk_private: ReinitializableSdkInstance,
) -> Result<()> {
    info!("=== Starting test_01b_spark_master_identity_public_key_user_setting ===");

    const MASTER_KEY: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const OTHER_MASTER_KEY: &str =
        "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

    let instance = persistent_sdk_private.build_sdk().await?;

    let initial_settings = instance.sdk.get_user_settings().await?;
    assert_eq!(initial_settings.spark_master_identity_public_key, None);
    assert!(initial_settings.spark_private_mode_enabled);

    // Designating a master identity must leave private mode untouched.
    instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: None,
            spark_master_identity_public_key: Some(SparkMasterIdentityPublicKey::Set {
                public_key: MASTER_KEY.to_string(),
            }),
        })
        .await?;
    let settings = instance.sdk.get_user_settings().await?;
    assert_eq!(
        settings.spark_master_identity_public_key.as_deref(),
        Some(MASTER_KEY)
    );
    assert!(settings.spark_private_mode_enabled);

    // Setting again replaces the previously designated key.
    instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: None,
            spark_master_identity_public_key: Some(SparkMasterIdentityPublicKey::Set {
                public_key: OTHER_MASTER_KEY.to_string(),
            }),
        })
        .await?;
    let settings = instance.sdk.get_user_settings().await?;
    assert_eq!(
        settings.spark_master_identity_public_key.as_deref(),
        Some(OTHER_MASTER_KEY)
    );

    instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: None,
            spark_master_identity_public_key: Some(SparkMasterIdentityPublicKey::Unset),
        })
        .await?;
    let settings = instance.sdk.get_user_settings().await?;
    assert_eq!(settings.spark_master_identity_public_key, None);
    assert!(settings.spark_private_mode_enabled);

    let err = instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: None,
            spark_master_identity_public_key: Some(SparkMasterIdentityPublicKey::Set {
                public_key: "not-a-public-key".to_string(),
            }),
        })
        .await
        .expect_err("a malformed master identity public key must be rejected");
    assert!(matches!(err, SdkError::InvalidInput(_)));

    info!("=== Test test_01b_spark_master_identity_public_key_user_setting PASSED ===");
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
                label: "SHELL".to_string(),
                token_identifier: SHELL_REGTEST_TOKEN_ID.to_string(),
            },
            StableBalanceToken {
                label: "BEAN".to_string(),
                token_identifier: BEAN_REGTEST_TOKEN_ID.to_string(),
            },
        ],
        default_active_label: Some("SHELL".to_string()),
        threshold_sats: Some(1000),
        max_slippage_bps: Some(500),
    });
    ReinitializableSdkInstance::new(
        cfg,
        tempfile::Builder::new()
            .prefix("breez-sdk-persistent-stable-balance")
            .tempdir()
            .unwrap(),
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
        settings.stable_balance_active_label.as_deref(),
        Some("SHELL")
    );

    // Unset the active ticker
    sdk_instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: Some(StableBalanceActiveLabel::Unset),
            spark_master_identity_public_key: None,
        })
        .await?;
    let settings = sdk_instance.sdk.get_user_settings().await?;
    assert_eq!(settings.stable_balance_active_label, None);

    // Set to BEAN
    sdk_instance
        .sdk
        .update_user_settings(UpdateUserSettingsRequest {
            spark_private_mode_enabled: None,
            stable_balance_active_label: Some(StableBalanceActiveLabel::Set {
                label: "BEAN".to_string(),
            }),
            spark_master_identity_public_key: None,
        })
        .await?;
    let settings = sdk_instance.sdk.get_user_settings().await?;
    assert_eq!(
        settings.stable_balance_active_label.as_deref(),
        Some("BEAN")
    );

    // Verify persistence across SDK reinitialization
    info!("=== Testing stable balance setting persistence across SDK reinitialization ===");
    drop(sdk_instance);

    let reinitialized = persistent_sdk_stable_balance.build_sdk().await?;
    let settings = reinitialized.sdk.get_user_settings().await?;
    assert_eq!(
        settings.stable_balance_active_label.as_deref(),
        Some("BEAN")
    );

    info!("=== Test test_02_stable_balance_user_setting PASSED ===");
    Ok(())
}
