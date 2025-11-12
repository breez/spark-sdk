use std::sync::Arc;

use spark_wallet::{SparkAddress, SparkWallet};

use crate::{
    BurnIssuerTokenRequest, CreateIssuerTokenRequest, FreezeIssuerTokenRequest,
    FreezeIssuerTokenResponse, MintIssuerTokenRequest, Payment, SdkError, Storage, TokenBalance,
    TokenMetadata, UnfreezeIssuerTokenRequest, UnfreezeIssuerTokenResponse,
    utils::token::map_and_persist_token_transaction,
};

#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BreezIssuerSdk {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
}

impl BreezIssuerSdk {
    pub fn new(spark_wallet: Arc<SparkWallet>, storage: Arc<dyn Storage>) -> Self {
        Self {
            spark_wallet,
            storage,
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl BreezIssuerSdk {
    /// Gets the issuer token balance
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `TokenBalance` - The balance of the issuer token
    /// * `SdkError` - If there was an error during the retrieval or no issuer token exists
    pub async fn get_issuer_token_balance(&self) -> Result<TokenBalance, SdkError> {
        Ok(self.spark_wallet.get_issuer_token_balance().await?.into())
    }

    /// Gets the issuer token metadata
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `TokenMetadata` - The metadata of the issuer token
    /// * `SdkError` - If there was an error during the retrieval or no issuer token exists
    pub async fn get_issuer_token_metadata(&self) -> Result<TokenMetadata, SdkError> {
        Ok(self.spark_wallet.get_issuer_token_metadata().await?.into())
    }

    /// Creates a new issuer token
    ///
    /// # Arguments
    ///
    /// * `request`: The request containing the token parameters
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `TokenMetadata` - The metadata of the created token
    /// * `SdkError` - If there was an error during the token creation
    pub async fn create_issuer_token(
        &self,
        request: CreateIssuerTokenRequest,
    ) -> Result<TokenMetadata, SdkError> {
        self.spark_wallet
            .create_issuer_token(
                &request.name,
                &request.ticker,
                request.decimals,
                request.is_freezable,
                request.max_supply,
            )
            .await?;
        self.get_issuer_token_metadata().await
    }

    /// Mints supply for the issuer token
    ///
    /// # Arguments
    ///
    /// * `request`: The request contiaining the amount of the supply to mint
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `Payment` - The payment representing the minting transaction
    /// * `SdkError` - If there was an error during the minting process
    pub async fn mint_issuer_token(
        &self,
        request: MintIssuerTokenRequest,
    ) -> Result<Payment, SdkError> {
        let token_transaction = self.spark_wallet.mint_issuer_token(request.amount).await?;
        map_and_persist_token_transaction(&self.spark_wallet, &self.storage, &token_transaction)
            .await
    }

    /// Burns supply of the issuer token
    ///
    /// # Arguments
    ///
    /// * `request`: The request containing the amount of the supply to burn
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `Payment` - The payment representing the burn transaction
    /// * `SdkError` - If there was an error during the burn process
    pub async fn burn_issuer_token(
        &self,
        request: BurnIssuerTokenRequest,
    ) -> Result<Payment, SdkError> {
        let token_transaction = self
            .spark_wallet
            .burn_issuer_token(request.amount, None)
            .await?;
        map_and_persist_token_transaction(&self.spark_wallet, &self.storage, &token_transaction)
            .await
    }

    /// Freezes tokens held at the specified address
    ///
    /// # Arguments
    ///
    /// * `request`: The request containing the spark address where the tokens to be frozen are held
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `FreezeIssuerTokenResponse` - The response containing details of the freeze operation
    /// * `SdkError` - If there was an error during the freeze process
    pub async fn freeze_issuer_token(
        &self,
        request: FreezeIssuerTokenRequest,
    ) -> Result<FreezeIssuerTokenResponse, SdkError> {
        let spark_address = request
            .address
            .parse::<SparkAddress>()
            .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
        Ok(self
            .spark_wallet
            .freeze_issuer_token(&spark_address)
            .await?
            .into())
    }

    /// Unfreezes tokens held at the specified address
    ///
    /// # Arguments
    ///
    /// * `request`: The request containing the spark address where the tokens to be unfrozen are held
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `UnfreezeIssuerTokenResponse` - The response containing details of the unfreeze operation
    /// * `SdkError` - If there was an error during the unfreeze process
    pub async fn unfreeze_issuer_token(
        &self,
        request: UnfreezeIssuerTokenRequest,
    ) -> Result<UnfreezeIssuerTokenResponse, SdkError> {
        let spark_address = request
            .address
            .parse::<SparkAddress>()
            .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
        Ok(self
            .spark_wallet
            .unfreeze_issuer_token(&spark_address)
            .await?
            .into())
    }
}
