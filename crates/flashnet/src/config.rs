use spark::Network;

#[derive(Clone, Debug)]
pub struct FlashnetConfig {
    pub base_url: String,
    pub network: Network,
}

impl FlashnetConfig {
    pub fn default_config(network: Network) -> Self {
        match network {
            Network::Mainnet => Self {
                base_url: "https://api.flashnet.xyz".to_string(),
                network,
            },
            Network::Regtest | Network::Testnet | Network::Signet => Self {
                base_url: "https://api.amm.makebitcoingreatagain.dev".to_string(),
                network,
            },
        }
    }
}
