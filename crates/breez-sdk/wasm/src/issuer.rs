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
pub struct BreezIssuerSdk {
    pub(crate) issuer_sdk: Rc<breez_sdk_spark::BreezIssuerSdk>,
}

#[wasm_bindgen]
impl BreezIssuerSdk {
    #[wasm_bindgen(js_name = "getIssuerTokenMetadata")]
    pub async fn get_issuer_token_metadata(&self) -> WasmResult<TokenMetadata> {
        Ok(self.issuer_sdk.get_issuer_token_metadata().await?.into())
    }

    #[wasm_bindgen(js_name = "getIssuerTokenBalance")]
    pub async fn get_issuer_token_balance(&self) -> WasmResult<TokenBalance> {
        Ok(self.issuer_sdk.get_issuer_token_balance().await?.into())
    }

    #[wasm_bindgen(js_name = "createIssuerToken")]
    pub async fn create_issuer_token(
        &self,
        request: CreateIssuerTokenRequest,
    ) -> WasmResult<TokenMetadata> {
        Ok(self
            .issuer_sdk
            .create_issuer_token(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "mintIssuerToken")]
    pub async fn mint_issuer_token(&self, request: MintIssuerTokenRequest) -> WasmResult<Payment> {
        Ok(self
            .issuer_sdk
            .mint_issuer_token(request.into())
            .await?
            .into())
    }

    #[wasm_bindgen(js_name = "burnIssuerToken")]
    pub async fn burn_issuer_token(&self, request: BurnIssuerTokenRequest) -> WasmResult<Payment> {
        Ok(self
            .issuer_sdk
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
            .issuer_sdk
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
            .issuer_sdk
            .unfreeze_issuer_token(request.into())
            .await?
            .into())
    }
}
