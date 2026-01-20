use std::collections::{HashMap, HashSet};

use tokio::sync::Mutex;
use tracing::{trace, warn};
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
    token_outputs: TokenOutputs,
    purpose: ReservationPurpose,
}

#[derive(Default)]
struct TokenOutputsState {
    available_token_outputs: HashMap<String, TokenOutputs>,
    reservations: HashMap<TokenOutputsReservationId, TokenOutputsEntry>,
}

#[macros::async_trait]
impl TokenOutputStore for InMemoryTokenOutputStore {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        // Update the pool of available token outputs
        token_outputs_state.available_token_outputs = token_outputs
            .iter()
            .map(|to| (to.metadata.identifier.clone(), to.clone()))
            .collect();

        // Reconcile reservations with the updated pool of token outputs
        for (id, reserved_token_outputs) in token_outputs_state.reservations.clone().iter() {
            // Get the token outputs for the reserved token identifier
            let Some(token_outputs) = token_outputs_state
                .available_token_outputs
                .get_mut(&reserved_token_outputs.token_outputs.metadata.identifier)
            else {
                // If the token outputs no longer exist, remove the reservation
                token_outputs_state.reservations.remove(id);
                continue;
            };
            // Filter out any reserved outputs no longer in the pool
            let output_ids = token_outputs.ids();
            let reserved_outputs = reserved_token_outputs
                .token_outputs
                .outputs
                .iter()
                .filter(|o| output_ids.contains(&o.output.id))
                .cloned()
                .collect::<Vec<_>>();
            if reserved_outputs.is_empty() {
                // If no reserved outputs exist anymore, remove the reservation
                token_outputs_state.reservations.remove(id);
                continue;
            }

            // Remove the reserved outputs from the pool outputs
            let reserved_output_ids = reserved_outputs
                .iter()
                .map(|o| o.output.id.clone())
                .collect::<HashSet<_>>();
            token_outputs
                .outputs
                .retain(|o| !reserved_output_ids.contains(&o.output.id));

            // Update the reservation with the reconciled outputs
            token_outputs_state.reservations.insert(
                id.clone(),
                TokenOutputsEntry {
                    token_outputs: TokenOutputs {
                        metadata: reserved_token_outputs.token_outputs.metadata.clone(),
                        outputs: reserved_outputs,
                    },
                    purpose: reserved_token_outputs.purpose,
                },
            );
        }

        trace!(
            "Updated {} token outputs in the local state",
            token_outputs.len()
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
            entry.available = token_outputs.outputs.clone();
        }

        for token_outputs_entry in token_outputs_state.reservations.values() {
            let entry = map
                .entry(
                    token_outputs_entry
                        .token_outputs
                        .metadata
                        .identifier
                        .clone(),
                )
                .or_insert(TokenOutputsPerStatus {
                    metadata: token_outputs_entry.token_outputs.metadata.clone(),
                    available: Vec::new(),
                    reserved_for_payment: Vec::new(),
                    reserved_for_swap: Vec::new(),
                });
            match token_outputs_entry.purpose {
                ReservationPurpose::Payment => {
                    entry
                        .reserved_for_payment
                        .extend(token_outputs_entry.token_outputs.outputs.iter().cloned());
                }
                ReservationPurpose::Swap => {
                    entry
                        .reserved_for_swap
                        .extend(token_outputs_entry.token_outputs.outputs.iter().cloned());
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
        // Check both available_token_outputs and reservations
        let (token_id, metadata) = match filter {
            GetTokenOutputsFilter::Identifier(token_id) => {
                // Try available outputs first
                if let Some(token_outputs) =
                    token_outputs_state.available_token_outputs.get(token_id)
                {
                    (token_id, token_outputs.metadata.clone())
                } else {
                    // If not in available, check reservations
                    let reservation = token_outputs_state
                        .reservations
                        .values()
                        .find(|r| r.token_outputs.metadata.identifier == token_id)
                        .ok_or(TokenOutputServiceError::Generic(
                            "Token outputs not found".to_string(),
                        ))?;
                    (token_id, reservation.token_outputs.metadata.clone())
                }
            }
            GetTokenOutputsFilter::IssuerPublicKey(issuer_pk) => {
                // Try available outputs first
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
                    // If not in available, check reservations
                    let reservation = token_outputs_state
                        .reservations
                        .values()
                        .find(|r| &r.token_outputs.metadata.issuer_public_key == issuer_pk)
                        .ok_or(TokenOutputServiceError::Generic(
                            "Token outputs not found".to_string(),
                        ))?;
                    (
                        reservation.token_outputs.metadata.identifier.as_str(),
                        reservation.token_outputs.metadata.clone(),
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
            result.available = token_outputs.outputs.clone();
        }

        for token_outputs_entry in token_outputs_state.reservations.values() {
            if token_outputs_entry.token_outputs.metadata.identifier == token_id {
                match token_outputs_entry.purpose {
                    ReservationPurpose::Payment => {
                        result
                            .reserved_for_payment
                            .extend(token_outputs_entry.token_outputs.outputs.iter().cloned());
                    }
                    ReservationPurpose::Swap => {
                        result
                            .reserved_for_swap
                            .extend(token_outputs_entry.token_outputs.outputs.iter().cloned());
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

        match token_outputs_state
            .available_token_outputs
            .get_mut(&token_outputs.metadata.identifier)
        {
            Some(existing_token_outputs) => {
                // Add only new outputs to the existing token outputs
                let existing_output_ids = existing_token_outputs.ids();
                token_outputs
                    .outputs
                    .iter()
                    .filter(|o| !existing_output_ids.contains(&o.output.id))
                    .for_each(|o| {
                        existing_token_outputs.outputs.push(o.clone());
                    });
            }
            None => {
                // Insert new token outputs
                token_outputs_state.available_token_outputs.insert(
                    token_outputs.metadata.identifier.clone(),
                    token_outputs.clone(),
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
            // Filter available outputs to only include preferred ones
            token_outputs
                .outputs
                .iter()
                .filter(|o| preferred_outputs.iter().any(|p| o.output.id == p.output.id))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            token_outputs.outputs.clone()
        };

        if let ReservationTarget::MinTotalValue(amount) = target
            && outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount
        {
            return Err(TokenOutputServiceError::InsufficientFunds);
        }

        let selected_outputs = if let ReservationTarget::MinTotalValue(amount) = target
            && let Some(output) = outputs.iter().find(|o| o.output.token_amount == amount)
        {
            // If there's an exact match, return it
            vec![output.clone()]
        } else {
            match selection_strategy {
                None | Some(SelectionStrategy::SmallestFirst) => {
                    // Sort outputs by amount, smallest first
                    outputs.sort_by_key(|o| o.output.token_amount);
                }
                Some(SelectionStrategy::LargestFirst) => {
                    // Sort outputs by amount, largest first
                    outputs.sort_by_key(|o| std::cmp::Reverse(o.output.token_amount));
                }
            }

            match target {
                ReservationTarget::MinTotalValue(amount) => {
                    // Select outputs to match the amount
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

                    // We should never get here, but just in case
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
        let reservation_token_outputs = TokenOutputs {
            metadata: token_outputs.metadata.clone(),
            outputs: selected_outputs.clone(),
        };

        // Remove selected outputs from the available pool
        let selected_output_ids = selected_outputs
            .iter()
            .map(|so| so.output.id.clone())
            .collect::<HashSet<_>>();
        token_outputs
            .outputs
            .retain(|to| !selected_output_ids.contains(&to.output.id));

        // Insert the reservation
        token_outputs_state.reservations.insert(
            reservation_id.clone(),
            TokenOutputsEntry {
                token_outputs: reservation_token_outputs.clone(),
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
        if let Some(reserved_token_outputs) = token_outputs_state.reservations.remove(id)
            && let Some(token_outputs) = token_outputs_state
                .available_token_outputs
                .get_mut(&reserved_token_outputs.token_outputs.metadata.identifier)
        {
            for output in reserved_token_outputs.token_outputs.outputs {
                token_outputs.outputs.push(output);
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
        if token_outputs_state.reservations.remove(id).is_none() {
            warn!("Tried to finalize a non existing reservation");
        }
        trace!("Finalized token outputs reservation: {}", id);
        Ok(())
    }
}


#[cfg(test)]
mod tests;
