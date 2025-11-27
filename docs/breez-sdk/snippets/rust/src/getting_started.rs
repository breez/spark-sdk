use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use breez_sdk_spark::*;
use tracing::info;

pub(crate) async fn init_sdk() -> Result<BreezSdk> {
    // ANCHOR: init-sdk
    // Construct the seed using mnemonic words or entropy bytes
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Connect to the SDK using the simplified connect method
    let sdk = connect(ConnectRequest {
        config,
        seed,
        storage_dir: "./.data".to_string(),
    })
    .await?;

    // ANCHOR_END: init-sdk
    Ok(sdk)
}

pub(crate) async fn getting_started_node_info(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: fetch-balance
    let info = sdk
        .get_info(GetInfoRequest {
            // ensure_synced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            ensure_synced: Some(false),
        })
        .await?;
    let balance_sats = info.balance_sats;
    // ANCHOR_END: fetch-balance
    info!("Balance: {balance_sats} sats");
    Ok(())
}

pub(crate) fn getting_started_logging(data_dir: String) -> Result<()> {
    // ANCHOR: logging
    let data_dir_path = PathBuf::from(&data_dir);
    fs::create_dir_all(data_dir_path)?;

    init_logging(Some(data_dir), None, None)?;
    // ANCHOR_END: logging
    Ok(())
}

// ANCHOR: add-event-listener
pub(crate) struct SdkEventListener {}

#[async_trait::async_trait]
impl EventListener for SdkEventListener {
    async fn on_event(&self, e: SdkEvent) {
        info!("Received event: {e:?}");
    }
}

pub(crate) async fn add_event_listener(
    sdk: &BreezSdk,
    listener: Box<SdkEventListener>,
) -> Result<String> {
    let listener_id = sdk.add_event_listener(listener).await;
    Ok(listener_id)
}
// ANCHOR_END: add-event-listener

// ANCHOR: remove-event-listener
pub(crate) async fn remove_event_listener(sdk: &BreezSdk, listener_id: &str) -> Result<()> {
    sdk.remove_event_listener(listener_id).await;
    Ok(())
}
// ANCHOR_END: remove-event-listener

// ANCHOR: disconnect
pub(crate) async fn disconnect(sdk: &BreezSdk) -> Result<()> {
    sdk.disconnect().await?;
    Ok(())
}
// ANCHOR_END: disconnect
