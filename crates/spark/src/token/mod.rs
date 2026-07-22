mod error;
mod service;
mod store;
mod token_service;

#[cfg(any(test, feature = "test-utils"))]
pub mod tests;

use std::collections::HashSet;

pub use error::TokenOutputServiceError;
pub use service::SynchronousTokenOutputService;
pub use store::InMemoryTokenOutputStore;
pub use token_service::*;

use bitcoin::secp256k1::PublicKey;
use platform_utils::time::SystemTime;
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

/// Token outputs together with the metadata describing the tokens they hold.
///
/// Outputs carry their own [`TokenOutput::token_identifier`], so a set may span
/// several tokens. `metadata` holds one entry per token appearing in `outputs`;
/// it is a separate lookup rather than a field on each output because metadata
/// is per token, not per output.
#[derive(Clone, Debug, Default)]
pub struct TokenOutputs {
    pub metadata: Vec<TokenMetadata>,
    pub outputs: Vec<TokenOutputWithPrevOut>,
}

impl TokenOutputs {
    pub fn single(metadata: TokenMetadata, outputs: Vec<TokenOutputWithPrevOut>) -> Self {
        Self {
            metadata: vec![metadata],
            outputs,
        }
    }

    pub fn prev_outpoints(&self) -> HashSet<(String, u32)> {
        self.outputs
            .iter()
            .map(|o| (o.prev_tx_hash.clone(), o.prev_tx_vout))
            .collect()
    }

    pub fn metadata_for(&self, token_identifier: &str) -> Option<&TokenMetadata> {
        self.metadata
            .iter()
            .find(|m| m.identifier == token_identifier)
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

/// Runs `f` and settles the reservation it depends on: finalized when `f` succeeds,
/// cancelled when it fails.
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

pub fn select_token_outputs_from(
    token_identifier: &str,
    mut outputs: Vec<TokenOutputWithPrevOut>,
    target: ReservationTarget,
    selection_strategy: Option<SelectionStrategy>,
) -> Result<Vec<TokenOutputWithPrevOut>, TokenOutputServiceError> {
    if let ReservationTarget::MinTotalValue(amount) = target
        && outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount
    {
        return Err(TokenOutputServiceError::InsufficientFunds {
            token_identifier: Some(token_identifier.to_string()),
        });
    }

    if let ReservationTarget::MinTotalValue(amount) = target
        && let Some(output) = outputs.iter().find(|o| o.output.token_amount == amount)
    {
        return Ok(vec![output.clone()]);
    }

    match selection_strategy {
        None | Some(SelectionStrategy::SmallestFirst) => {
            outputs.sort_by_key(|o| o.output.token_amount);
        }
        Some(SelectionStrategy::LargestFirst) => {
            outputs.sort_by_key(|o| std::cmp::Reverse(o.output.token_amount));
        }
    }

    match target {
        ReservationTarget::MinTotalValue(amount) => {
            let mut selected_outputs = Vec::new();
            let mut remaining_amount = amount;
            for output in outputs {
                if remaining_amount == 0 {
                    break;
                }
                selected_outputs.push(output.clone());
                remaining_amount = remaining_amount.saturating_sub(output.output.token_amount);
            }

            if remaining_amount > 0 {
                return Err(TokenOutputServiceError::InsufficientFunds {
                    token_identifier: Some(token_identifier.to_string()),
                });
            }

            Ok(selected_outputs)
        }
        ReservationTarget::MaxOutputCount(count) => {
            outputs.truncate(count);
            Ok(outputs)
        }
    }
}

#[macros::async_trait]
pub trait TokenOutputStore: Send + Sync {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &TokenOutputs,
        refresh_started_at: SystemTime,
    ) -> Result<(), TokenOutputServiceError>;

    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError>;

    /// Returns just the spendable per-token balances paired with their metadata.
    /// Default impl falls through to `list_tokens_outputs`; storage backends that
    /// can compute the aggregate server-side should override.
    async fn get_token_balances(
        &self,
    ) -> Result<Vec<(TokenMetadata, u128)>, TokenOutputServiceError> {
        Ok(self
            .list_tokens_outputs()
            .await?
            .into_iter()
            .map(|t| {
                let balance = t.balance();
                (t.metadata, balance)
            })
            .collect())
    }

    async fn get_token_outputs(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenOutputsPerStatus, TokenOutputServiceError>;

    /// Atomically removes spent outputs (identified by their previous
    /// transaction coordinates) and inserts new outputs into the store.
    ///
    /// Removed outputs are marked as spent so that a concurrent refresh will
    /// not re-add them.
    async fn update_token_outputs(
        &self,
        outputs_to_remove: &[(String, u32)],
        outputs_to_add: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError>;

    /// Reserves outputs covering every target in one atomic step, so a transaction
    /// spanning several tokens holds a single reservation rather than one per token.
    async fn reserve_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError>;

    async fn select_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputs, TokenOutputServiceError>;

    /// Reserves the given outpoints, which may belong to different tokens.
    async fn reserve_token_outputs_by_outpoints(
        &self,
        outpoints: &[(String, u32)],
        purpose: ReservationPurpose,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError>;

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError>;

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError>;

    /// Returns the current time from the store's clock.
    ///
    /// For in-memory stores this returns `SystemTime::now()`. For database-backed
    /// stores this queries the database server time, avoiding clock skew between
    /// the application and database servers.
    async fn now(&self) -> Result<SystemTime, TokenOutputServiceError>;
}

#[macros::async_trait]
pub trait TokenOutputService: Send + Sync {
    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError>;

    async fn get_token_balances(
        &self,
    ) -> Result<Vec<(TokenMetadata, u128)>, TokenOutputServiceError>;

    async fn refresh_tokens_outputs(&self) -> Result<(), TokenOutputServiceError>;

    async fn get_token_metadata(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenMetadata, TokenOutputServiceError>;

    /// Atomically removes spent outputs (identified by their previous
    /// transaction coordinates) and inserts new outputs into the store.
    ///
    /// Removed outputs are marked as spent so that a concurrent refresh will
    /// not re-add them.
    async fn update_token_outputs(
        &self,
        outputs_to_remove: &[(String, u32)],
        outputs_to_add: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError>;

    /// Reserves outputs covering every target in one atomic step, so a transaction
    /// spanning several tokens holds a single reservation rather than one per token.
    async fn reserve_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError>;

    async fn select_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputs, TokenOutputServiceError>;

    /// Reserves the given outpoints, which may belong to different tokens.
    async fn reserve_token_outputs_by_outpoints(
        &self,
        outpoints: &[(String, u32)],
        purpose: ReservationPurpose,
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
