mod error;
mod service;
mod store;

use std::collections::HashSet;

pub use error::TokenOutputServiceError;
pub use service::SynchronousTokenOutputService;
pub use store::InMemoryTokenOutputStore;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use tracing::error;

pub enum GetTokenOutputsFilter<'a> {
    Identifier(&'a str),
    IssuerPublicKey(&'a PublicKey),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenMetadata {
    pub identifier: String,
    pub issuer_public_key: PublicKey,
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub max_supply: u128,
    pub is_freezable: bool,
    pub creation_entity_public_key: Option<PublicKey>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct TokenOutput {
    pub id: String,
    pub owner_public_key: PublicKey,
    pub revocation_commitment: String,
    pub withdraw_bond_sats: u64,
    pub withdraw_relative_block_locktime: u64,
    pub token_public_key: Option<PublicKey>,
    pub token_identifier: String,
    pub token_amount: u128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenOutputWithPrevOut {
    pub output: TokenOutput,
    pub prev_tx_hash: String,
    pub prev_tx_vout: u32,
}

#[derive(Clone, Debug)]
pub struct TokenOutputs {
    pub metadata: TokenMetadata,
    pub outputs: Vec<TokenOutputWithPrevOut>,
}

impl TokenOutputs {
    pub fn ids(&self) -> HashSet<String> {
        self.outputs.iter().map(|o| o.output.id.clone()).collect()
    }
}

pub type TokenOutputsReservationId = String;

#[derive(Clone, Debug)]
pub struct TokenOutputsReservation {
    pub id: TokenOutputsReservationId,
    pub token_outputs: TokenOutputs,
}

impl TokenOutputsReservation {
    pub fn new(id: TokenOutputsReservationId, token_outputs: TokenOutputs) -> Self {
        Self { id, token_outputs }
    }
}

pub async fn with_reserved_token_outputs<F, R, E>(
    token_output_service: &dyn TokenOutputService,
    f: F,
    reservation: &TokenOutputsReservation,
) -> Result<R, E>
where
    F: Future<Output = Result<R, E>>,
{
    match f.await {
        Ok(r) => {
            if let Err(e) = token_output_service
                .finalize_reservation(&reservation.id)
                .await
            {
                error!("Failed to finalize reservation: {e:?}");
            }
            Ok(r)
        }
        Err(e) => {
            if let Err(e) = token_output_service
                .cancel_reservation(&reservation.id)
                .await
            {
                error!("Failed to cancel reservation: {e:?}");
            }
            Err(e)
        }
    }
}

#[macros::async_trait]
pub trait TokenOutputStore: Send + Sync {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
    ) -> Result<(), TokenOutputServiceError>;

    async fn list_tokens_outputs(&self) -> Result<Vec<TokenOutputs>, TokenOutputServiceError>;

    async fn get_token_outputs(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<Option<TokenOutputs>, TokenOutputServiceError>;

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError>;

    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError>;

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError>;

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError>;
}

#[macros::async_trait]
pub trait TokenOutputService: Send + Sync {
    async fn list_tokens_outputs(&self) -> Result<Vec<TokenOutputs>, TokenOutputServiceError>;

    async fn refresh_tokens_outputs(&self) -> Result<(), TokenOutputServiceError>;

    async fn get_token_metadata(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<Option<TokenMetadata>, TokenOutputServiceError>;

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError>;

    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError>;

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError>;

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError>;
}
