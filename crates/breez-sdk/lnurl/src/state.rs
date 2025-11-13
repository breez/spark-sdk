use std::sync::Arc;

pub struct State<DB> {
    pub db: DB,
    pub wallet: Arc<spark_wallet::SparkWallet>,
    pub scheme: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub include_spark_address: bool,
    pub domain_validator: Arc<dyn domain_validator::DomainValidator>,
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
            include_spark_address: self.include_spark_address,
            domain_validator: Arc::clone(&self.domain_validator),
            ca_cert: self.ca_cert.clone(),
        }
    }
}
