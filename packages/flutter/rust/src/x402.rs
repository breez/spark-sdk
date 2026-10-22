use std::sync::Arc;

use breez_sdk_spark::{PayX402Request, PayX402Response, SdkError};

pub struct X402 {
    pub(crate) x402: Arc<breez_sdk_spark::X402>,
}

impl X402 {
    pub async fn pay(&self, request: PayX402Request) -> Result<PayX402Response, SdkError> {
        self.x402.pay(request).await
    }
}
