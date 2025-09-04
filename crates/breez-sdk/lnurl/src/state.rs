use std::sync::Arc;

use breez_sdk_spark::BreezSdk;

pub struct State<DB> {
    pub db: Arc<DB>,
    pub sdk: Arc<BreezSdk>,
    pub domain: String,
    pub min_sendable: u64,
    pub max_sendable: u64,
}

impl<DB> Clone for State<DB> {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            sdk: Arc::clone(&self.sdk),
            domain: self.domain.clone(),
            min_sendable: self.min_sendable,
            max_sendable: self.max_sendable,
        }
    }
}
