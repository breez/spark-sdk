use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

pub(crate) fn configure_sdk() -> Result<()> {
    // ANCHOR: max-deposit-claim-fee
    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Disable automatic claiming
    config.max_deposit_claim_fee = None;

    // Set a maximum feerate of 10 sat/vB
    config.max_deposit_claim_fee = Some(Fee::Rate { sat_per_vbyte: 10 });

    // Set a maximum fee of 1000 sat
    config.max_deposit_claim_fee = Some(Fee::Fixed { amount: 1000 });
    // ANCHOR_END: max-deposit-claim-fee
    info!("Config: {:?}", config);
    Ok(())
}

pub(crate) fn configure_private_enabled_default() -> Result<()> {
    // ANCHOR: private-enabled-default
    // Disable Spark private mode by default
    let mut config = default_config(Network::Mainnet);
    config.private_enabled_default = false;
    // ANCHOR_END: private-enabled-default
    info!("Config: {:?}", config);
    Ok(())
}