use std::sync::Arc;

use breez_sdk_spark::{
    BurnTokensRequest, CreateTokenRequest, FreezeTokensRequest, FreezeTokensResponse,
    GetIssuerTokenBalanceResponse, MintTokensRequest, Payment, SdkError, TokenMetadata,
    UnfreezeTokensRequest, UnfreezeTokensResponse,
};

pub struct BreezIssuerSdk {
    pub(crate) issuer_sdk: Arc<breez_sdk_spark::BreezIssuerSdk>,
}

impl BreezIssuerSdk {
    pub async fn get_issuer_token_metadata(&self) -> Result<TokenMetadata, SdkError> {
        self.issuer_sdk.get_issuer_token_metadata().await
    }

    pub async fn get_issuer_token_balance(
        &self,
    ) -> Result<GetIssuerTokenBalanceResponse, SdkError> {
        self.issuer_sdk.get_issuer_token_balance().await
    }

    pub async fn create_issuer_token(
        &self,
        request: CreateTokenRequest,
    ) -> Result<TokenMetadata, SdkError> {
        self.issuer_sdk.create_issuer_token(request).await
    }

    pub async fn mint_issuer_token(&self, request: MintTokensRequest) -> Result<Payment, SdkError> {
        self.issuer_sdk.mint_issuer_token(request).await
    }

    pub async fn burn_issuer_token(&self, request: BurnTokensRequest) -> Result<Payment, SdkError> {
        self.issuer_sdk.burn_issuer_token(request).await
    }

    pub async fn freeze_issuer_token(
        &self,
        request: FreezeTokensRequest,
    ) -> Result<FreezeTokensResponse, SdkError> {
        self.issuer_sdk.freeze_issuer_token(request).await
    }

    pub async fn unfreeze_issuer_token(
        &self,
        request: UnfreezeTokensRequest,
    ) -> Result<UnfreezeTokensResponse, SdkError> {
        self.issuer_sdk.unfreeze_issuer_token(request).await
    }
}
