pub mod data_sync;
pub mod docker;
pub mod lnurl;
pub mod socks5;
pub mod ssp_fault;

use anyhow::Result;
use breez_sdk_spark::{StableBalanceConfig, StableBalanceToken};
use rand::RngCore;

/// Token identifiers for regtest
pub const BEAN_REGTEST_TOKEN_ID: &str =
    "btknrt1muwlm2aeur2jhe4pkuh7v08jjaleqgeu69c5sk7p7qfhkywg7nkqerzl06";
pub const SHELL_REGTEST_TOKEN_ID: &str =
    "btknrt1ra8lrwpqgqfz7gcy3gfcucaw3fh62tp3d6qkjxafx0cnxm5gmd3q0xy27c";

/// USDB ("Bitcoin USD", 6 decimals) token identifier on mainnet. Used by the
/// env-gated mainnet conversion tests (override with `MAINNET_TEST_TOKEN_ID`).
pub const USDB_MAINNET_TOKEN_ID: &str =
    "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87";

pub fn random_mnemonic() -> Result<String> {
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    Ok(bip39::Mnemonic::from_entropy(&entropy)?.to_string())
}

pub fn stable_balance_config() -> StableBalanceConfig {
    StableBalanceConfig {
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
    }
}
