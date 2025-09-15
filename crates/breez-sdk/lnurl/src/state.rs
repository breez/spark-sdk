use std::{collections::HashSet, sync::Arc};

pub struct State<DB> {
    pub db: DB,
    pub wallet: Arc<spark_wallet::SparkWallet>,
    pub scheme: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub domains: HashSet<String>,
    pub ca_cert: Option<Vec<u8>>,
}

impl<DB> Clone for State<DB>
where
    DB: Clone,
{
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            wallet: Arc::clone(&self.wallet),
            scheme: self.scheme.clone(),
            min_sendable: self.min_sendable,
            max_sendable: self.max_sendable,
            domains: self.domains.clone(),
            ca_cert: self.ca_cert.clone(),
        }
    }
}
