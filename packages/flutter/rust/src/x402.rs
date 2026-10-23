use std::sync::Arc;

use breez_sdk_spark::{
    CreateX402ChallengeRequest, CreateX402ChallengeResponse, PayX402Request, PayX402Response,
    SdkError, SettleX402Request, SettleX402Response,
};

pub struct X402 {
    pub(crate) x402: Arc<breez_sdk_spark::X402>,
}

impl X402 {
    pub async fn pay(&self, request: PayX402Request) -> Result<PayX402Response, SdkError> {
        self.x402.pay(request).await
    }

    pub async fn create_challenge(
        &self,
        request: CreateX402ChallengeRequest,
    ) -> Result<CreateX402ChallengeResponse, SdkError> {
        self.x402.create_challenge(request).await
    }

    pub async fn settle(&self, request: SettleX402Request) -> Result<SettleX402Response, SdkError> {
        self.x402.settle(request).await
    }
}
