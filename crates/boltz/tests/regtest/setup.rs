//! Regtest configuration and health check.

use boltz::{AlchemyConfig, BoltzConfig};

/// Regtest Boltz API URL.
const REGTEST_API_URL: &str = "http://localhost:9001";
/// Anvil RPC URL (from Docker stack).
const REGTEST_ANVIL_RPC: &str = "http://localhost:8545";
/// Anvil chain ID (matches web app regtest config).
const REGTEST_CHAIN_ID: u64 = 33;

/// Build a `BoltzConfig` for the local regtest environment.
pub fn regtest_config() -> BoltzConfig {
    BoltzConfig {
        api_url: REGTEST_API_URL.to_string(),
        alchemy_config: AlchemyConfig {
            api_key: "unused-in-regtest".to_string(),
            gas_policy_id: "unused-in-regtest".to_string(),
        },
        arbitrum_rpc_url: REGTEST_ANVIL_RPC.to_string(),
        chain_id: REGTEST_CHAIN_ID,
        referral_id: "regtest".to_string(),
        slippage_bps: 100,
    }
}

/// Seed bytes for regtest testing (deterministic, not secret).
pub fn regtest_seed() -> Vec<u8> {
    // "abandon" x11 + "about" mnemonic → always the same seed
    let mnemonic = bip39::Mnemonic::parse_normalized(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .expect("valid test mnemonic");
    mnemonic.to_seed("").to_vec()
}
