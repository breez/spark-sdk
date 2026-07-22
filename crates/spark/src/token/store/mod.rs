use std::collections::{HashMap, HashSet};

use platform_utils::time::SystemTime;
use tokio::sync::Mutex;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::token::{
    GetTokenOutputsFilter, ReservationPurpose, ReservationTarget, SelectionStrategy,
    TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
    TokenOutputsPerStatus, TokenOutputsReservation, TokenOutputsReservationId,
    select_token_outputs_from,
};

#[derive(Default)]
pub struct InMemoryTokenOutputStore {
    token_outputs: Mutex<TokenOutputsState>,
}

#[derive(Clone)]
struct TokenOutputsEntry {
    /// Metadata for every token the reserved outputs span.
    metadata: Vec<crate::token::TokenMetadata>,
    stored_outputs: Vec<StoredTokenOutput>,
    purpose: ReservationPurpose,
}

impl TokenOutputsEntry {
    fn holds_token(&self, token_identifier: &str) -> bool {
        self.stored_outputs
            .iter()
            .any(|s| s.output.output.token_identifier == token_identifier)
    }

    /// The reserved outputs belonging to one token.
    fn outputs_for(&self, token_identifier: &str) -> impl Iterator<Item = TokenOutputWithPrevOut> {
        self.stored_outputs
            .iter()
            .filter(move |s| s.output.output.token_identifier == token_identifier)
            .map(|s| s.output.clone())
    }
}

fn validate_target(target: ReservationTarget) -> Result<(), TokenOutputServiceError> {
    match target {
        ReservationTarget::MinTotalValue(0) => Err(TokenOutputServiceError::Generic(
            "Amount to reserve must be greater than zero".to_string(),
        )),
        ReservationTarget::MaxOutputCount(0) => Err(TokenOutputServiceError::Generic(
            "Count to reserve must be greater than zero".to_string(),
        )),
        _ => Ok(()),
    }
}

/// A token output bundled with the timestamp it was added to the pool.
#[derive(Clone)]
struct StoredTokenOutput {
    output: TokenOutputWithPrevOut,
    added_at: SystemTime,
}

/// Canonical key for an on-chain output: parent tx hash + vout. Used instead of the server
/// `TokenOutput.id` because the v3 broadcast response (`FinalTokenOutput`) carries no id, so
/// id-keyed dedup would let the same output appear twice in the pool.
type OutPoint = (String, u32);

#[derive(Default)]
pub(crate) struct TokenOutputsState {
    /// Available (unreserved) token outputs, keyed by token identifier.
    /// Each value maps an `OutPoint` to its StoredTokenOutput for timestamp tracking.
    available_token_outputs: HashMap<String, AvailableTokenOutputs>,
    reservations: HashMap<TokenOutputsReservationId, TokenOutputsEntry>,
    /// Outpoints that have been finalized (spent) with their spent timestamp.
    /// Prevents re-adding during refresh. Cleaned up when entry is older than refresh_started_at.
    spent_outpoints: HashMap<OutPoint, SystemTime>,
    /// Timestamp of when the most recent swap finalization completed.
    /// Used to detect if a refresh started before a swap finished.
    last_swap_completed_at: Option<SystemTime>,
}

/// Available outputs for a specific token, with per-output timestamps.
#[derive(Clone)]
struct AvailableTokenOutputs {
    metadata: crate::token::TokenMetadata,
    outputs: HashMap<OutPoint, StoredTokenOutput>,
}

impl AvailableTokenOutputs {
    fn outpoints(&self) -> HashSet<OutPoint> {
        self.outputs.keys().cloned().collect()
    }

    fn output_vec(&self) -> Vec<TokenOutputWithPrevOut> {
        self.outputs.values().map(|s| s.output.clone()).collect()
    }
}

fn outpoint_of(o: &TokenOutputWithPrevOut) -> OutPoint {
    (o.prev_tx_hash.clone(), o.prev_tx_vout)
}

/// Picks outputs from one token's available pool. `preferred_outputs` narrows the
/// candidates; entries belonging to other tokens simply do not match, so callers
/// may pass one list covering every token.
fn select_available(
    pool: &AvailableTokenOutputs,
    preferred_outputs: Option<&[TokenOutputWithPrevOut]>,
    target: ReservationTarget,
    selection_strategy: Option<SelectionStrategy>,
) -> Result<Vec<TokenOutputWithPrevOut>, TokenOutputServiceError> {
    let candidates = match preferred_outputs {
        Some(preferred) => {
            let preferred_outpoints: HashSet<OutPoint> =
                preferred.iter().map(outpoint_of).collect();
            pool.output_vec()
                .into_iter()
                .filter(|o| preferred_outpoints.contains(&outpoint_of(o)))
                .collect::<Vec<_>>()
        }
        None => pool.output_vec(),
    };
    select_token_outputs_from(
        &pool.metadata.identifier,
        candidates,
        target,
        selection_strategy,
    )
}

#[macros::async_trait]
impl TokenOutputStore for InMemoryTokenOutputStore {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &TokenOutputs,
        refresh_started_at: SystemTime,
    ) -> Result<(), TokenOutputServiceError> {
        let mut state = self.token_outputs.lock().await;

        // Skip if swap is active or completed during this refresh
        let has_active_swap = state
            .reservations
            .values()
            .any(|entry| matches!(entry.purpose, ReservationPurpose::Swap));
        let swap_completed_during_refresh = state
            .last_swap_completed_at
            .is_some_and(|completed_at| completed_at >= refresh_started_at);
        if has_active_swap || swap_completed_during_refresh {
            debug!(
                "Skipping set_tokens_outputs: active_swap={has_active_swap}, \
                 swap_completed_during_refresh={swap_completed_during_refresh}"
            );
            return Ok(());
        }

        // Remove spent entries that operators have had time to process
        state
            .spent_outpoints
            .retain(|_, spent_at| *spent_at >= refresh_started_at);

        // Save old pools before replacing
        let old_available = std::mem::take(&mut state.available_token_outputs);

        // Build new pools from refresh data, excluding spent outputs
        let now = SystemTime::now();
        let spent_outpoints: HashSet<OutPoint> = state.spent_outpoints.keys().cloned().collect();
        for output in &token_outputs.outputs {
            let identifier = &output.output.token_identifier;
            let Some(metadata) = token_outputs.metadata_for(identifier) else {
                warn!("No metadata for token {identifier}; skipping output");
                continue;
            };
            let entry = state
                .available_token_outputs
                .entry(identifier.clone())
                .or_insert_with(|| AvailableTokenOutputs {
                    metadata: metadata.clone(),
                    outputs: HashMap::new(),
                });
            entry.metadata = metadata.clone();
            let outpoint = outpoint_of(output);
            if !spent_outpoints.contains(&outpoint) {
                entry.outputs.insert(
                    outpoint,
                    StoredTokenOutput {
                        output: output.clone(),
                        added_at: now,
                    },
                );
            }
        }

        // Re-add outputs from old state that were added after the refresh started
        // and aren't in the refresh data (they weren't available when refresh collected data).
        let mut preserved_count = 0u32;
        for old_token_outputs in old_available.values() {
            for (outpoint, stored) in &old_token_outputs.outputs {
                if stored.added_at >= refresh_started_at {
                    // Check if this output already exists in the new data
                    let already_exists = state
                        .available_token_outputs
                        .values()
                        .any(|ato| ato.outputs.contains_key(outpoint));
                    if !already_exists {
                        let entry = state
                            .available_token_outputs
                            .entry(old_token_outputs.metadata.identifier.clone())
                            .or_insert_with(|| AvailableTokenOutputs {
                                metadata: old_token_outputs.metadata.clone(),
                                outputs: HashMap::new(),
                            });
                        entry.outputs.insert(outpoint.clone(), stored.clone());
                        preserved_count += 1;
                    }
                }
            }
        }

        // Reconcile reservations with the updated pool of token outputs
        // A reservation may span tokens, so reconcile it against every pool it touches.
        for (id, reserved_entry) in state.reservations.clone().iter() {
            let mut reserved_stored = Vec::new();
            for stored in &reserved_entry.stored_outputs {
                let Some(pool) = state
                    .available_token_outputs
                    .get_mut(&stored.output.output.token_identifier)
                else {
                    continue;
                };
                if pool.outpoints().contains(&outpoint_of(&stored.output)) {
                    pool.outputs.remove(&outpoint_of(&stored.output));
                    reserved_stored.push(stored.clone());
                }
            }
            if reserved_stored.is_empty() {
                state.reservations.remove(id);
                continue;
            }

            state.reservations.insert(
                id.clone(),
                TokenOutputsEntry {
                    metadata: reserved_entry.metadata.clone(),
                    stored_outputs: reserved_stored,
                    purpose: reserved_entry.purpose,
                },
            );
        }

        trace!(
            "Updated {} token outputs in the local state ({} preserved from previous state)",
            token_outputs.outputs.len(),
            preserved_count
        );
        Ok(())
    }

    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError> {
        let token_outputs_state = self.token_outputs.lock().await;

        let mut map = HashMap::new();

        for (token_id, token_outputs) in token_outputs_state.available_token_outputs.iter() {
            let entry = map
                .entry(token_id.clone())
                .or_insert(TokenOutputsPerStatus {
                    metadata: token_outputs.metadata.clone(),
                    available: Vec::new(),
                    reserved_for_payment: Vec::new(),
                    reserved_for_swap: Vec::new(),
                });
            entry.available = token_outputs.output_vec();
        }

        // A reservation may span tokens, so it contributes to one bucket per token.
        for token_outputs_entry in token_outputs_state.reservations.values() {
            for metadata in &token_outputs_entry.metadata {
                if !token_outputs_entry.holds_token(&metadata.identifier) {
                    continue;
                }
                let entry =
                    map.entry(metadata.identifier.clone())
                        .or_insert(TokenOutputsPerStatus {
                            metadata: metadata.clone(),
                            available: Vec::new(),
                            reserved_for_payment: Vec::new(),
                            reserved_for_swap: Vec::new(),
                        });
                let reserved = token_outputs_entry.outputs_for(&metadata.identifier);
                match token_outputs_entry.purpose {
                    ReservationPurpose::Payment => entry.reserved_for_payment.extend(reserved),
                    ReservationPurpose::Swap => entry.reserved_for_swap.extend(reserved),
                }
            }
        }

        Ok(map.into_values().collect())
    }

    async fn get_token_outputs(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenOutputsPerStatus, TokenOutputServiceError> {
        let token_outputs_state = self.token_outputs.lock().await;

        // Find the matching token identifier and metadata, looking in the available
        // pools first and then in any reservation holding that token.
        let reserved_metadata = |pred: &dyn Fn(&crate::token::TokenMetadata) -> bool| {
            token_outputs_state
                .reservations
                .values()
                .flat_map(|r| r.metadata.iter())
                .find(|m| pred(m))
                .cloned()
        };

        let metadata = match filter {
            GetTokenOutputsFilter::Identifier(token_id) => token_outputs_state
                .available_token_outputs
                .get(token_id)
                .map(|to| to.metadata.clone())
                .or_else(|| reserved_metadata(&|m| m.identifier == token_id)),
            GetTokenOutputsFilter::IssuerPublicKey(issuer_pk) => token_outputs_state
                .available_token_outputs
                .values()
                .find(|to| &to.metadata.issuer_public_key == issuer_pk)
                .map(|to| to.metadata.clone())
                .or_else(|| reserved_metadata(&|m| &m.issuer_public_key == issuer_pk)),
        }
        .ok_or(TokenOutputServiceError::Generic(
            "Token outputs not found".to_string(),
        ))?;
        let token_id = metadata.identifier.clone();
        let token_id = token_id.as_str();

        let mut result = TokenOutputsPerStatus {
            metadata,
            available: Vec::new(),
            reserved_for_payment: Vec::new(),
            reserved_for_swap: Vec::new(),
        };

        if let Some(token_outputs) = token_outputs_state.available_token_outputs.get(token_id) {
            result.available = token_outputs.output_vec();
        }

        for token_outputs_entry in token_outputs_state.reservations.values() {
            let reserved = token_outputs_entry.outputs_for(token_id);
            match token_outputs_entry.purpose {
                ReservationPurpose::Payment => result.reserved_for_payment.extend(reserved),
                ReservationPurpose::Swap => result.reserved_for_swap.extend(reserved),
            }
        }

        Ok(result)
    }

    async fn update_token_outputs(
        &self,
        outputs_to_remove: &[(String, u32)],
        outputs_to_add: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        let mut state = self.token_outputs.lock().await;
        let now = SystemTime::now();

        // 1. Remove spent outputs by (prev_tx_hash, prev_tx_vout) and mark as spent.
        for (tx_hash, vout) in outputs_to_remove {
            let outpoint = (tx_hash.clone(), *vout);
            for available in state.available_token_outputs.values_mut() {
                if available.outputs.remove(&outpoint).is_some() {
                    break;
                }
            }
            state.spent_outpoints.insert(outpoint, now);
        }

        // 2. Insert new outputs, each filed under its own token.
        for output in &outputs_to_add.outputs {
            let outpoint = outpoint_of(output);
            if state.spent_outpoints.remove(&outpoint).is_some() {
                trace!(
                    "Removed outpoint {}:{} from spent_outpoints (receiving it back)",
                    outpoint.0, outpoint.1
                );
            }

            let identifier = &output.output.token_identifier;
            let Some(metadata) = outputs_to_add.metadata_for(identifier) else {
                warn!("No metadata for token {identifier}; skipping output");
                continue;
            };
            state
                .available_token_outputs
                .entry(identifier.clone())
                .or_insert_with(|| AvailableTokenOutputs {
                    metadata: metadata.clone(),
                    outputs: HashMap::new(),
                })
                .outputs
                .entry(outpoint)
                .or_insert_with(|| StoredTokenOutput {
                    output: output.clone(),
                    added_at: now,
                });
        }

        if !outputs_to_add.outputs.is_empty() {
            trace!(
                "Inserted {} token outputs into the local state",
                outputs_to_add.outputs.len()
            );
        }

        if !outputs_to_remove.is_empty() {
            trace!(
                "Removed {} token outputs from the local state",
                outputs_to_remove.len()
            );
        }

        Ok(())
    }

    async fn reserve_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        if targets.is_empty() {
            return Err(TokenOutputServiceError::Generic(
                "No reservation targets provided".to_string(),
            ));
        }
        for (_, target) in targets {
            validate_target(*target)?;
        }

        // Every token is selected and reserved under this one lock, so the whole
        // reservation either happens or does not.
        let mut state = self.token_outputs.lock().await;

        let mut selected = Vec::new();
        for (token_identifier, target) in targets {
            let Some(pool) = state.available_token_outputs.get(token_identifier) else {
                return Err(TokenOutputServiceError::Generic(format!(
                    "Token outputs not found for identifier: {token_identifier}"
                )));
            };
            selected.push((
                pool.metadata.clone(),
                select_available(
                    pool,
                    preferred_outputs.as_deref(),
                    *target,
                    selection_strategy,
                )?,
            ));
        }

        let mut metadata = Vec::with_capacity(selected.len());
        let mut outputs = Vec::new();
        let mut stored_selected = Vec::new();
        for (token_metadata, token_selected) in selected {
            let pool = state
                .available_token_outputs
                .get_mut(&token_metadata.identifier)
                .ok_or_else(|| {
                    TokenOutputServiceError::Generic(format!(
                        "Token outputs not found for identifier: {}",
                        token_metadata.identifier
                    ))
                })?;
            for output in &token_selected {
                if let Some(stored) = pool.outputs.remove(&outpoint_of(output)) {
                    stored_selected.push(stored);
                }
            }
            metadata.push(token_metadata);
            outputs.extend(token_selected);
        }

        let reservation_id = Uuid::now_v7().to_string();
        state.reservations.insert(
            reservation_id.clone(),
            TokenOutputsEntry {
                metadata: metadata.clone(),
                stored_outputs: stored_selected,
                purpose,
            },
        );

        Ok(TokenOutputsReservation::new(
            reservation_id,
            TokenOutputs { metadata, outputs },
        ))
    }

    async fn select_token_outputs(
        &self,
        targets: &[(String, ReservationTarget)],
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputs, TokenOutputServiceError> {
        if targets.is_empty() {
            return Err(TokenOutputServiceError::Generic(
                "No selection targets provided".to_string(),
            ));
        }
        for (_, target) in targets {
            validate_target(*target)?;
        }

        let state = self.token_outputs.lock().await;

        let mut metadata = Vec::with_capacity(targets.len());
        let mut outputs = Vec::new();
        for (token_identifier, target) in targets {
            let Some(pool) = state.available_token_outputs.get(token_identifier) else {
                return Err(TokenOutputServiceError::Generic(format!(
                    "Token outputs not found for identifier: {token_identifier}"
                )));
            };
            outputs.extend(select_available(
                pool,
                preferred_outputs.as_deref(),
                *target,
                selection_strategy,
            )?);
            metadata.push(pool.metadata.clone());
        }

        Ok(TokenOutputs { metadata, outputs })
    }

    async fn reserve_token_outputs_by_outpoints(
        &self,
        outpoints: &[(String, u32)],
        purpose: ReservationPurpose,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        if outpoints.is_empty() {
            return Err(TokenOutputServiceError::Generic(
                "No outpoints provided".to_string(),
            ));
        }

        let mut state = self.token_outputs.lock().await;
        let wanted: HashSet<OutPoint> = outpoints.iter().cloned().collect();

        // The outpoints may belong to different tokens, so search every pool.
        let mut metadata = Vec::new();
        let mut outputs = Vec::new();
        let mut stored_selected = Vec::new();
        for pool in state.available_token_outputs.values_mut() {
            let matching: Vec<OutPoint> = pool
                .outputs
                .keys()
                .filter(|op| wanted.contains(*op))
                .cloned()
                .collect();
            if matching.is_empty() {
                continue;
            }
            metadata.push(pool.metadata.clone());
            for outpoint in matching {
                if let Some(stored) = pool.outputs.remove(&outpoint) {
                    outputs.push(stored.output.clone());
                    stored_selected.push(stored);
                }
            }
        }

        if outputs.len() != wanted.len() {
            // Put back whatever was taken; the reservation is all-or-nothing.
            for stored in stored_selected {
                if let Some(pool) = state
                    .available_token_outputs
                    .get_mut(&stored.output.output.token_identifier)
                {
                    pool.outputs.insert(outpoint_of(&stored.output), stored);
                }
            }
            return Err(TokenOutputServiceError::InsufficientFunds {
                token_identifier: None,
            });
        }

        let reservation_id = Uuid::now_v7().to_string();
        state.reservations.insert(
            reservation_id.clone(),
            TokenOutputsEntry {
                metadata: metadata.clone(),
                stored_outputs: stored_selected,
                purpose,
            },
        );

        Ok(TokenOutputsReservation::new(
            reservation_id,
            TokenOutputs { metadata, outputs },
        ))
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        if let Some(reserved_entry) = token_outputs_state.reservations.remove(id) {
            for stored in reserved_entry.stored_outputs {
                if let Some(pool) = token_outputs_state
                    .available_token_outputs
                    .get_mut(&stored.output.output.token_identifier)
                {
                    pool.outputs.insert(outpoint_of(&stored.output), stored);
                }
            }
        }
        trace!("Canceled token outputs reservation: {}", id);
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        if let Some(entry) = token_outputs_state.reservations.remove(id) {
            // Mark all outputs from this reservation as spent to prevent re-adding during refresh
            let now = SystemTime::now();
            for stored in &entry.stored_outputs {
                token_outputs_state
                    .spent_outpoints
                    .insert(outpoint_of(&stored.output), now);
            }

            // If this was a swap reservation, record completion time.
            if matches!(entry.purpose, ReservationPurpose::Swap) {
                token_outputs_state.last_swap_completed_at = Some(now);
            }
        } else {
            warn!("Tried to finalize a non existing reservation");
        }
        trace!("Finalized token outputs reservation: {}", id);
        Ok(())
    }

    async fn now(&self) -> Result<SystemTime, TokenOutputServiceError> {
        Ok(SystemTime::now())
    }
}

#[cfg(test)]
mod tests;
