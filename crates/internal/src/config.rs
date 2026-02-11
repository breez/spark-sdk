use spark_wallet::Network;

#[derive(Clone, Debug)]
pub struct MempoolConfig {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl MempoolConfig {
    pub fn default_for_network(network: Network) -> Self {
        match network {
            Network::Mainnet => Self {
                url: "https://mempool.space/api".to_string(),
                username: None,
                password: None,
            },
            _ => Self {
                url: "https://regtest-mempool.us-west-2.sparkinfra.net/api".to_string(),
                username: Some("spark-sdk".to_string()),
                password: Some("mCMk1JqlBNtetUNy".to_string()),
            },
        }
    }

    pub fn from_env(network: Network) -> Self {
        let default = Self::default_for_network(network);
        Self {
            url: std::env::var("SPARK_MEMPOOL_URL").unwrap_or(default.url),
            username: std::env::var("SPARK_MEMPOOL_USERNAME")
                .ok()
                .or(default.username),
            password: std::env::var("SPARK_MEMPOOL_PASSWORD")
                .ok()
                .or(default.password),
        }
    }
}
