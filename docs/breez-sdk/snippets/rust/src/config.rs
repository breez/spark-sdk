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
    config.max_deposit_claim_fee = Some(MaxFee::Rate { sat_per_vbyte: 10 });

    // Set a maximum fee of 1000 sat
    config.max_deposit_claim_fee = Some(MaxFee::Fixed { amount: 1000 });

    // Set the maximum fee to the fastest network recommended fee at the time of claim
    // with a leeway of 1 sats/vbyte
    config.max_deposit_claim_fee = Some(MaxFee::NetworkRecommended {
        leeway_sat_per_vbyte: 1,
    });
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

pub(crate) fn configure_optimization_configuration() -> Result<()> {
    // ANCHOR: optimization-configuration
    let mut config = default_config(Network::Mainnet);
    config.optimization_config = OptimizationConfig {
        auto_enabled: true,
        multiplicity: 1,
    };
    // ANCHOR_END: optimization-configuration
    info!("Config: {:?}", config);
    Ok(())
}

pub(crate) fn configure_stable_balance() -> Result<()> {
    // ANCHOR: stable-balance-config
    let mut config = default_config(Network::Mainnet);

    // Enable stable balance with auto-conversion to a specific token
    config.stable_balance_config = Some(StableBalanceConfig {
        token_identifier: "<token_identifier>".to_string(),
        threshold_sats: Some(10_000),
        max_slippage_bps: Some(100),
        reserved_sats: Some(1_000),
    });
    // ANCHOR_END: stable-balance-config
    info!("Config: {:?}", config);
    Ok(())
}
