use spark::Network;
use spark_wallet::PublicKey;

#[derive(Clone, Debug)]
pub struct FlashnetConfig {
    pub base_url: String,
    pub network: Network,
    pub integrator_config: Option<IntegratorConfig>,
}

#[derive(Clone, Debug)]
pub struct IntegratorConfig {
    pub pubkey: PublicKey,
    pub fee_bps: u32,
}

/// Configuration for the Flashnet Orchestra (cross-chain) API.
///
/// `base_url` and `api_key` are fetched at connect time from Breez server so
/// the key can be rotated and revoked without an SDK release. The key is
/// intentionally omitted from `Debug` output.
#[derive(Clone)]
pub struct OrchestraConfig {
    pub base_url: String,
    pub api_key: String,
}

impl std::fmt::Debug for OrchestraConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrchestraConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl FlashnetConfig {
    pub fn default_config(network: Network, integrator_config: Option<IntegratorConfig>) -> Self {
        match network {
            Network::Mainnet => Self {
                base_url: "https://api.flashnet.xyz".to_string(),
                network,
                integrator_config,
            },
            Network::Regtest | Network::Testnet | Network::Signet => Self {
                base_url: "https://api.amm.makebitcoingreatagain.dev".to_string(),
                network,
                integrator_config,
            },
        }
    }
}
