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
