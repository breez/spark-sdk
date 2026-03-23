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

pub(crate) fn configure_spark_config() -> Result<()> {
    // ANCHOR: spark-config
    let mut config = default_config(Network::Mainnet);

    // Connect to a custom Spark environment
    config.spark_config = Some(SparkConfig {
        coordinator_identifier: "0000000000000000000000000000000000000000000000000000000000000001"
            .to_string(),
        threshold: 2,
        signing_operators: vec![
            SparkSigningOperator {
                id: 0,
                identifier:
                    "0000000000000000000000000000000000000000000000000000000000000001"
                        .to_string(),
                address: "https://0.spark.example.com".to_string(),
                identity_public_key:
                    "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651"
                        .to_string(),
            },
            SparkSigningOperator {
                id: 1,
                identifier:
                    "0000000000000000000000000000000000000000000000000000000000000002"
                        .to_string(),
                address: "https://1.spark.example.com".to_string(),
                identity_public_key:
                    "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23"
                        .to_string(),
            },
            SparkSigningOperator {
                id: 2,
                identifier:
                    "0000000000000000000000000000000000000000000000000000000000000003"
                        .to_string(),
                address: "https://2.spark.example.com".to_string(),
                identity_public_key:
                    "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853"
                        .to_string(),
            },
        ],
        ssp_config: SparkSspConfig {
            base_url: "https://api.example.com".to_string(),
            identity_public_key:
                "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5"
                    .to_string(),
            schema_endpoint: Some("graphql/spark/rc".to_string()),
        },
        expected_withdraw_bond_sats: 10_000,
        expected_withdraw_relative_block_locktime: 1_000,
    });
    // ANCHOR_END: spark-config
    info!("Config: {:?}", config);
    Ok(())
}
