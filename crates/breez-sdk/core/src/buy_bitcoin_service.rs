use std::sync::Arc;

use async_trait::async_trait;

use crate::error::SdkError;

/// Service for initiating Bitcoin purchases via external providers
#[async_trait]
pub trait BuyBitcoinService: Send + Sync {
    async fn buy_bitcoin(
        &self,
        address: String,
        locked_amount_sat: Option<u64>,
        max_amount_sat: Option<u64>,
        redirect_url: Option<String>,
    ) -> Result<String, SdkError>;
}

/// MoonPay-based Bitcoin purchase service
pub struct MoonpayBuyBitcoinService {
    breez_server: Arc<breez_sdk_common::breez_server::BreezServer>,
}

impl MoonpayBuyBitcoinService {
    pub fn new(breez_server: Arc<breez_sdk_common::breez_server::BreezServer>) -> Self {
        Self { breez_server }
    }
}

#[async_trait]
impl BuyBitcoinService for MoonpayBuyBitcoinService {
    async fn buy_bitcoin(
        &self,
        address: String,
        locked_amount_sat: Option<u64>,
        max_amount_sat: Option<u64>,
        redirect_url: Option<String>,
    ) -> Result<String, SdkError> {
        use breez_sdk_common::buy::{BuyBitcoinProviderApi, moonpay::MoonpayProvider};

        let provider = MoonpayProvider::new(Arc::clone(&self.breez_server));
        provider
            .buy_bitcoin(address, locked_amount_sat, max_amount_sat, redirect_url)
            .await
            .map_err(|e| SdkError::Generic(format!("Failed to create buy bitcoin URL: {e}")))
    }
}
