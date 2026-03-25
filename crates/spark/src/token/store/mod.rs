use std::collections::{HashMap, HashSet};

use platform_utils::time::SystemTime;
use tokio::sync::Mutex;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::token::{
    GetTokenOutputsFilter, ReservationPurpose, ReservationTarget, SelectionStrategy,
    TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
    TokenOutputsPerStatus, TokenOutputsReservation, TokenOutputsReservationId,
};

#[derive(Default)]
pub struct InMemoryTokenOutputStore {
    token_outputs: Mutex<TokenOutputsState>,
}

#[derive(Clone)]
struct TokenOutputsEntry {
    metadata: crate::token::TokenMetadata,
    stored_outputs: Vec<StoredTokenOutput>,
    purpose: ReservationPurpose,
}

/// A token output bundled with the timestamp it was added to the pool.
#[derive(Clone)]
struct StoredTokenOutput {
    output: TokenOutputWithPrevOut,
    added_at: SystemTime,
}

#[derive(Default)]
pub(crate) struct TokenOutputsState {
    /// Available (unreserved) token outputs, keyed by token identifier.
    /// Each value maps output ID to StoredTokenOutput for timestamp tracking.
    available_token_outputs: HashMap<String, AvailableTokenOutputs>,
    reservations: HashMap<TokenOutputsReservationId, TokenOutputsEntry>,
    /// Output IDs that have been finalized (spent) with their spent timestamp.
    /// Prevents re-adding during refresh. Cleaned up when entry is older than refresh_started_at.
    spent_output_ids: HashMap<String, SystemTime>,
    /// Timestamp of when the most recent swap finalization completed.
    /// Used to detect if a refresh started before a swap finished.
    last_swap_completed_at: Option<SystemTime>,
}

/// Available outputs for a specific token, with per-output timestamps.
#[derive(Clone)]
struct AvailableTokenOutputs {
    metadata: crate::token::TokenMetadata,
    outputs: HashMap<String, StoredTokenOutput>,
}

impl AvailableTokenOutputs {
    fn ids(&self) -> HashSet<String> {
        self.outputs.keys().cloned().collect()
    }

    fn output_vec(&self) -> Vec<TokenOutputWithPrevOut> {
        self.outputs.values().map(|s| s.output.clone()).collect()
    }
}

#[macros::async_trait]
impl TokenOutputStore for InMemoryTokenOutputStore {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
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
            .spent_output_ids
            .retain(|_, spent_at| *spent_at >= refresh_started_at);

        // Save old pools before replacing
        let old_available = std::mem::take(&mut state.available_token_outputs);

        // Build new pools from refresh data, excluding spent outputs
        let now = SystemTime::now();
        let spent_ids: HashSet<String> = state.spent_output_ids.keys().cloned().collect();
        for to in token_outputs {
            let identifier = to.metadata.identifier.clone();
            let entry = state
                .available_token_outputs
                .entry(identifier)
                .or_insert_with(|| AvailableTokenOutputs {
                    metadata: to.metadata.clone(),
                    outputs: HashMap::new(),
                });
            entry.metadata = to.metadata.clone();
            for output in &to.outputs {
                if !spent_ids.contains(&output.output.id) {
                    entry.outputs.insert(
                        output.output.id.clone(),
                        StoredTokenOutput {
                            output: output.clone(),
                            added_at: now,
                        },
                    );
                }
            }
        }

        // Re-add outputs from old state that were added after the refresh started
        // and aren't in the refresh data (they weren't available when refresh collected data).
        let mut preserved_count = 0u32;
        for old_token_outputs in old_available.values() {
            for (output_id, stored) in &old_token_outputs.outputs {
                if stored.added_at >= refresh_started_at {
                    // Check if this output already exists in the new data
                    let already_exists = state
                        .available_token_outputs
                        .values()
                        .any(|ato| ato.outputs.contains_key(output_id));
                    if !already_exists {
                        let entry = state
                            .available_token_outputs
                            .entry(old_token_outputs.metadata.identifier.clone())
                            .or_insert_with(|| AvailableTokenOutputs {
                                metadata: old_token_outputs.metadata.clone(),
                                outputs: HashMap::new(),
                            });
                        entry.outputs.insert(output_id.clone(), stored.clone());
                        preserved_count += 1;
                    }
                }
            }
        }

        // Reconcile reservations with the updated pool of token outputs
        for (id, reserved_entry) in state.reservations.clone().iter() {
            let Some(token_outputs) = state
                .available_token_outputs
                .get_mut(&reserved_entry.metadata.identifier)
            else {
                state.reservations.remove(id);
                continue;
            };
            let output_ids = token_outputs.ids();
            let reserved_stored = reserved_entry
                .stored_outputs
                .iter()
                .filter(|s| output_ids.contains(&s.output.output.id))
                .cloned()
                .collect::<Vec<_>>();
            if reserved_stored.is_empty() {
                state.reservations.remove(id);
                continue;
            }

            // Remove the reserved outputs from the pool
            let reserved_output_ids = reserved_stored
                .iter()
                .map(|s| s.output.output.id.clone())
                .collect::<HashSet<_>>();
            token_outputs
                .outputs
                .retain(|id, _| !reserved_output_ids.contains(id));

            // Update the reservation with the reconciled outputs
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
            token_outputs.len(),
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

        for token_outputs_entry in token_outputs_state.reservations.values() {
            let entry = map
                .entry(token_outputs_entry.metadata.identifier.clone())
                .or_insert(TokenOutputsPerStatus {
                    metadata: token_outputs_entry.metadata.clone(),
                    available: Vec::new(),
                    reserved_for_payment: Vec::new(),
                    reserved_for_swap: Vec::new(),
                });
            match token_outputs_entry.purpose {
                ReservationPurpose::Payment => {
                    entry.reserved_for_payment.extend(
                        token_outputs_entry
                            .stored_outputs
                            .iter()
                            .map(|s| s.output.clone()),
                    );
                }
                ReservationPurpose::Swap => {
                    entry.reserved_for_swap.extend(
                        token_outputs_entry
                            .stored_outputs
                            .iter()
                            .map(|s| s.output.clone()),
                    );
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

        // Find the matching token identifier and metadata
        let (token_id, metadata) = match filter {
            GetTokenOutputsFilter::Identifier(token_id) => {
                if let Some(token_outputs) =
                    token_outputs_state.available_token_outputs.get(token_id)
                {
                    (token_id, token_outputs.metadata.clone())
                } else {
                    let reservation = token_outputs_state
                        .reservations
                        .values()
                        .find(|r| r.metadata.identifier == token_id)
                        .ok_or(TokenOutputServiceError::Generic(
                            "Token outputs not found".to_string(),
                        ))?;
                    (token_id, reservation.metadata.clone())
                }
            }
            GetTokenOutputsFilter::IssuerPublicKey(issuer_pk) => {
                if let Some(token_outputs) = token_outputs_state
                    .available_token_outputs
                    .values()
                    .find(|to| &to.metadata.issuer_public_key == issuer_pk)
                {
                    (
                        token_outputs.metadata.identifier.as_str(),
                        token_outputs.metadata.clone(),
                    )
                } else {
                    let reservation = token_outputs_state
                        .reservations
                        .values()
                        .find(|r| &r.metadata.issuer_public_key == issuer_pk)
                        .ok_or(TokenOutputServiceError::Generic(
                            "Token outputs not found".to_string(),
                        ))?;
                    (
                        reservation.metadata.identifier.as_str(),
                        reservation.metadata.clone(),
                    )
                }
            }
        };

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
            if token_outputs_entry.metadata.identifier == token_id {
                match token_outputs_entry.purpose {
                    ReservationPurpose::Payment => {
                        result.reserved_for_payment.extend(
                            token_outputs_entry
                                .stored_outputs
                                .iter()
                                .map(|s| s.output.clone()),
                        );
                    }
                    ReservationPurpose::Swap => {
                        result.reserved_for_swap.extend(
                            token_outputs_entry
                                .stored_outputs
                                .iter()
                                .map(|s| s.output.clone()),
                        );
                    }
                }
            }
        }

        Ok(result)
    }

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        let now = SystemTime::now();

        // Remove inserted output IDs from spent_output_ids (output returned to us)
        for output in &token_outputs.outputs {
            if token_outputs_state
                .spent_output_ids
                .remove(&output.output.id)
                .is_some()
            {
                trace!(
                    "Removed output {} from spent_output_ids (receiving it back)",
                    output.output.id
                );
            }
        }

        match token_outputs_state
            .available_token_outputs
            .get_mut(&token_outputs.metadata.identifier)
        {
            Some(existing_token_outputs) => {
                for o in &token_outputs.outputs {
                    if !existing_token_outputs.outputs.contains_key(&o.output.id) {
                        existing_token_outputs.outputs.insert(
                            o.output.id.clone(),
                            StoredTokenOutput {
                                output: o.clone(),
                                added_at: now,
                            },
                        );
                    }
                }
            }
            None => {
                let mut outputs_map = HashMap::new();
                for o in &token_outputs.outputs {
                    outputs_map.insert(
                        o.output.id.clone(),
                        StoredTokenOutput {
                            output: o.clone(),
                            added_at: now,
                        },
                    );
                }
                token_outputs_state.available_token_outputs.insert(
                    token_outputs.metadata.identifier.clone(),
                    AvailableTokenOutputs {
                        metadata: token_outputs.metadata.clone(),
                        outputs: outputs_map,
                    },
                );
            }
        }

        trace!(
            "Inserted {} token outputs into the local state",
            token_outputs.outputs.len()
        );
        Ok(())
    }

    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: ReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        match target {
            ReservationTarget::MinTotalValue(amount) => {
                if amount == 0 {
                    return Err(TokenOutputServiceError::Generic(
                        "Amount to reserve must be greater than zero".to_string(),
                    ));
                }
            }
            ReservationTarget::MaxOutputCount(count) => {
                if count == 0 {
                    return Err(TokenOutputServiceError::Generic(
                        "Count to reserve must be greater than zero".to_string(),
                    ));
                }
            }
        }

        let mut token_outputs_state = self.token_outputs.lock().await;
        let Some(token_outputs) = token_outputs_state
            .available_token_outputs
            .get_mut(token_identifier)
        else {
            return Err(TokenOutputServiceError::Generic(format!(
                "Token outputs not found for identifier: {}",
                token_identifier
            )));
        };

        let mut outputs = if let Some(preferred_outputs) = preferred_outputs {
            token_outputs
                .output_vec()
                .into_iter()
                .filter(|o| preferred_outputs.iter().any(|p| o.output.id == p.output.id))
                .collect::<Vec<_>>()
        } else {
            token_outputs.output_vec()
        };

        if let ReservationTarget::MinTotalValue(amount) = target
            && outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount
        {
            return Err(TokenOutputServiceError::InsufficientFunds);
        }

        let selected_outputs = if let ReservationTarget::MinTotalValue(amount) = target
            && let Some(output) = outputs.iter().find(|o| o.output.token_amount == amount)
        {
            vec![output.clone()]
        } else {
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
                        remaining_amount =
                            remaining_amount.saturating_sub(output.output.token_amount);
                    }

                    if remaining_amount > 0 {
                        return Err(TokenOutputServiceError::InsufficientFunds);
                    }

                    selected_outputs
                }
                ReservationTarget::MaxOutputCount(count) => {
                    outputs.truncate(count);
                    outputs
                }
            }
        };

        let reservation_id = Uuid::now_v7().to_string();

        // Collect stored outputs with their original added_at timestamps
        let stored_selected: Vec<StoredTokenOutput> = selected_outputs
            .iter()
            .filter_map(|o| token_outputs.outputs.get(&o.output.id).cloned())
            .collect();

        let metadata = token_outputs.metadata.clone();
        let reservation_token_outputs = TokenOutputs {
            metadata: metadata.clone(),
            outputs: selected_outputs.clone(),
        };

        // Remove selected outputs from the available pool
        let selected_output_ids = selected_outputs
            .iter()
            .map(|so| so.output.id.clone())
            .collect::<HashSet<_>>();
        token_outputs
            .outputs
            .retain(|id, _| !selected_output_ids.contains(id));

        // Insert the reservation (with original timestamps preserved)
        token_outputs_state.reservations.insert(
            reservation_id.clone(),
            TokenOutputsEntry {
                metadata,
                stored_outputs: stored_selected,
                purpose,
            },
        );

        Ok(TokenOutputsReservation::new(
            reservation_id,
            reservation_token_outputs,
        ))
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        if let Some(reserved_entry) = token_outputs_state.reservations.remove(id)
            && let Some(token_outputs) = token_outputs_state
                .available_token_outputs
                .get_mut(&reserved_entry.metadata.identifier)
        {
            for stored in reserved_entry.stored_outputs {
                token_outputs
                    .outputs
                    .insert(stored.output.output.id.clone(), stored);
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
                    .spent_output_ids
                    .insert(stored.output.output.id.clone(), now);
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
