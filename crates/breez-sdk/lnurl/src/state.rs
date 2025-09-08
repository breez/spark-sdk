use std::sync::Arc;

use spark_wallet::DefaultSigner;

pub struct State<DB> {
    pub db: DB,
    pub wallet: Arc<spark_wallet::SparkWallet<DefaultSigner>>,
    pub scheme: String,
    pub domain: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
}

impl<DB> Clone for State<DB>
where
    DB: Clone,
{
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            wallet: Arc::clone(&self.wallet),
            domain: self.domain.clone(),
            scheme: self.scheme.clone(),
            min_sendable: self.min_sendable,
            max_sendable: self.max_sendable,
        }
    }
}
