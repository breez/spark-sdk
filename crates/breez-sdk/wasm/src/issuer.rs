use std::rc::Rc;

use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    error::WasmResult,
    models::{
        Payment, TokenBalance, TokenMetadata,
        issuer::{
            BurnIssuerTokenRequest, CreateIssuerTokenRequest, FreezeIssuerTokenRequest,
            FreezeIssuerTokenResponse, MintIssuerTokenRequest, UnfreezeIssuerTokenRequest,
            UnfreezeIssuerTokenResponse,
        },
    },
};

#[wasm_bindgen]
pub struct TokenIssuer {
    pub(crate) token_issuer: Rc<breez_sdk_spark::TokenIssuer>,
}

#[wasm_bindgen]
impl TokenIssuer {
    #[wasm_bindgen(js_name = "getIssuerTokenMetadata")]
    pub async fn get_issuer_token_metadata(&self) -> WasmResult<TokenMetadata> {
        Ok(self.token_issuer.get_issuer_token_metadata().await?.into())
    }

    #[wasm_bindgen(js_name = "getIssuerTokenBalance")]
    pub async fn get_issuer_token_balance(&self) -> WasmResult<TokenBalance> {
        Ok(self.token_issuer.get_issuer_token_balance().await?.into())
    }

    #[wasm_bindgen(js_name = "createIssuerToken")]
    pub async fn create_issuer_token(
        &self,
        request: CreateIssuerTokenRequest,
    ) -> WasmResult<TokenMetadata> {
        Ok(self
            .token_issuer
            .create_issuer_token(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "mintIssuerToken")]
    pub async fn mint_issuer_token(&self, request: MintIssuerTokenRequest) -> WasmResult<Payment> {
        Ok(self
            .token_issuer
            .mint_issuer_token(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "burnIssuerToken")]
    pub async fn burn_issuer_token(&self, request: BurnIssuerTokenRequest) -> WasmResult<Payment> {
        Ok(self
            .token_issuer
            .burn_issuer_token(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "freezeIssuerToken")]
    pub async fn freeze_issuer_token(
        &self,
        request: FreezeIssuerTokenRequest,
    ) -> WasmResult<FreezeIssuerTokenResponse> {
        Ok(self
            .token_issuer
            .freeze_issuer_token(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "unfreezeIssuerToken")]
    pub async fn unfreeze_issuer_token(
        &self,
        request: UnfreezeIssuerTokenRequest,
    ) -> WasmResult<UnfreezeIssuerTokenResponse> {
        Ok(self
            .token_issuer
            .unfreeze_issuer_token(request.into())
            .await?
            .into())
    }
}
