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

#[derive(Clone, Debug)]
pub struct TokenOutputsPerStatus {
    pub metadata: TokenMetadata,
    pub available: Vec<TokenOutputWithPrevOut>,
    /// Tokens reserved for payment. Should be excluded from balance since they are being spent.
    pub reserved_for_payment: Vec<TokenOutputWithPrevOut>,
    /// Tokens reserved for swap. Should be included in balance since they will be received back.
    pub reserved_for_swap: Vec<TokenOutputWithPrevOut>,
}

impl TokenOutputsPerStatus {
    pub fn available_balance(&self) -> u128 {
        self.available.iter().map(|o| o.output.token_amount).sum()
    }

    pub fn reserved_for_swap_balance(&self) -> u128 {
        self.reserved_for_swap
            .iter()
            .map(|o| o.output.token_amount)
            .sum()
    }

    pub fn balance(&self) -> u128 {
        self.available_balance() + self.reserved_for_swap_balance()
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

#[derive(Clone, Debug, Copy)]
pub enum SelectionStrategy {
    SmallestFirst,
    LargestFirst,
}

#[derive(Clone, Debug, Copy)]
pub enum ReservationTarget {
    /// The minimum total value of the token outputs to reserve.
    MinTotalValue(u128),
    /// The maximum number of token outputs to reserve.
    MaxOutputCount(usize),
}

#[derive(Clone, Debug, Copy)]
pub enum ReservationPurpose {
    /// Leaves being used for a payment - excluded from balance since they
    /// are about to be spent.
    Payment,
    /// Leaves will be swapped. Included in balance since we will receive
    /// the same amount back.
    Swap,
}

#[macros::async_trait]
pub trait TokenOutputStore: Send + Sync {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
    ) -> Result<(), TokenOutputServiceError>;

    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError>;

    async fn get_token_outputs(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenOutputsPerStatus, TokenOutputServiceError>;

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError>;

    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
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
    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError>;

    async fn refresh_tokens_outputs(&self) -> Result<(), TokenOutputServiceError>;

    async fn get_token_metadata(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenMetadata, TokenOutputServiceError>;

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError>;

    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
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
