use std::sync::Arc;

use breez_sdk_spark::{
    BurnIssuerTokenRequest, CreateIssuerTokenRequest, FreezeIssuerTokenRequest,
    FreezeIssuerTokenResponse, MintIssuerTokenRequest, Payment, SdkError, TokenBalance,
    TokenMetadata, UnfreezeIssuerTokenRequest, UnfreezeIssuerTokenResponse,
};

pub struct TokenIssuer {
    pub(crate) token_issuer: Arc<breez_sdk_spark::TokenIssuer>,
}

impl TokenIssuer {
    pub async fn get_issuer_token_metadata(&self) -> Result<TokenMetadata, SdkError> {
        self.token_issuer.get_issuer_token_metadata().await
    }

    pub async fn get_issuer_token_balance(&self) -> Result<TokenBalance, SdkError> {
        self.token_issuer.get_issuer_token_balance().await
    }

    pub async fn create_issuer_token(
        &self,
        request: CreateIssuerTokenRequest,
    ) -> Result<TokenMetadata, SdkError> {
        self.token_issuer.create_issuer_token(request).await
    }

    pub async fn mint_issuer_token(
        &self,
        request: MintIssuerTokenRequest,
    ) -> Result<Payment, SdkError> {
        self.token_issuer.mint_issuer_token(request).await
    }

    pub async fn burn_issuer_token(
        &self,
        request: BurnIssuerTokenRequest,
    ) -> Result<Payment, SdkError> {
        self.token_issuer.burn_issuer_token(request).await
    }

    pub async fn freeze_issuer_token(
        &self,
        request: FreezeIssuerTokenRequest,
    ) -> Result<FreezeIssuerTokenResponse, SdkError> {
        self.token_issuer.freeze_issuer_token(request).await
    }

    pub async fn unfreeze_issuer_token(
        &self,
        request: UnfreezeIssuerTokenRequest,
    ) -> Result<UnfreezeIssuerTokenResponse, SdkError> {
        self.token_issuer.unfreeze_issuer_token(request).await
    }
}
