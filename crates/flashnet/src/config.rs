use spark::Network;
use spark_wallet::PublicKey;

#[derive(Clone, Debug)]
pub struct FlashnetConfig {
    pub base_url: String,
    pub network: Network,
    pub integrator_config: Option<IntegratorConfig>,
    pub orchestra: OrchestraConfig,
}

#[derive(Clone, Debug)]
pub struct IntegratorConfig {
    pub pubkey: PublicKey,
    pub fee_bps: u32,
}

/// Configuration for the Flashnet Orchestra (cross-chain) API.
///
/// `api_key` is bundled by default with a Breez-owned key so integrators do
/// not need to supply one. Use `default_for_network` to construct. The key is
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

// Default Breez-owned Orchestra API keys per network. Placeholder values —
// replace with real keys before release. See CLAUDE.md / deployment docs.
const ORCHESTRA_API_KEY_MAINNET: &str = "fn_breez_mainnet_placeholder";
const ORCHESTRA_API_KEY_TESTNET: &str = "fn_breez_testnet_placeholder";

const ORCHESTRA_BASE_URL: &str = "https://orchestration.flashnet.xyz";

impl OrchestraConfig {
    pub fn default_for_network(network: Network) -> Self {
        let api_key = match network {
            Network::Mainnet => ORCHESTRA_API_KEY_MAINNET,
            Network::Regtest | Network::Testnet | Network::Signet => ORCHESTRA_API_KEY_TESTNET,
        };
        Self {
            base_url: ORCHESTRA_BASE_URL.to_string(),
            api_key: api_key.to_string(),
        }
    }
}

impl FlashnetConfig {
    pub fn default_config(network: Network, integrator_config: Option<IntegratorConfig>) -> Self {
        let orchestra = OrchestraConfig::default_for_network(network);
        match network {
            Network::Mainnet => Self {
                base_url: "https://api.flashnet.xyz".to_string(),
                network,
                integrator_config,
                orchestra,
            },
            Network::Regtest | Network::Testnet | Network::Signet => Self {
                base_url: "https://api.amm.makebitcoingreatagain.dev".to_string(),
                network,
                integrator_config,
                orchestra,
            },
        }
    }
}
