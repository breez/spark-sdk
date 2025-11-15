use std::collections::HashMap;

use bitcoin::secp256k1::PublicKey;
use tokio::sync::Mutex;
use tracing::{trace, warn};
use uuid::Uuid;

use crate::token::{
    TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
    TokenOutputsReservation, TokenOutputsReservationId,
};

#[derive(Default)]
pub struct InMemoryTokenOutputStore {
    token_outputs: Mutex<TokenOutputsState>,
}

#[derive(Default)]
struct TokenOutputsState {
    token_outputs: HashMap<String, TokenOutputs>,
    reservations: HashMap<TokenOutputsReservationId, TokenOutputs>,
}

#[macros::async_trait]
impl TokenOutputStore for InMemoryTokenOutputStore {
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        // Update the pool of available token outputs
        token_outputs_state.token_outputs = token_outputs
            .iter()
            .map(|to| (to.metadata.identifier.clone(), to.clone()))
            .collect();

        // Reconcile reservations with the updated pool of token outputs
        for (id, reserved_token_outputs) in token_outputs_state.reservations.clone().iter() {
            // Get the token outputs for the reserved token identifier
            let Some(token_outputs) = token_outputs_state
                .token_outputs
                .get_mut(&reserved_token_outputs.metadata.identifier)
            else {
                // If the token outputs no longer exist, remove the reservation
                token_outputs_state.reservations.remove(id);
                continue;
            };
            // Filter out any reserved outputs no longer in the pool
            let output_ids = token_outputs.ids();
            let reserved_outputs = reserved_token_outputs
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
                .collect::<Vec<_>>();
            token_outputs
                .outputs
                .retain(|o| !reserved_output_ids.contains(&o.output.id));

            // Update the reservation with the reconciled outputs
            token_outputs_state.reservations.insert(
                id.clone(),
                TokenOutputs {
                    metadata: reserved_token_outputs.metadata.clone(),
                    outputs: reserved_outputs,
                },
            );
        }

        trace!(
            "Updated {} token outputs in the local state",
            token_outputs.len()
        );
        Ok(())
    }

    async fn list_tokens_outputs(&self) -> Result<Vec<TokenOutputs>, TokenOutputServiceError> {
        let token_outputs_state = self.token_outputs.lock().await;
        Ok(token_outputs_state
            .token_outputs
            .values()
            .cloned()
            .collect())
    }

    async fn get_token_outputs(
        &self,
        token_identifier: Option<&str>,
        issuer_public_key: Option<&PublicKey>,
    ) -> Result<Option<TokenOutputs>, TokenOutputServiceError> {
        let token_outputs_state = self.token_outputs.lock().await;
        if let Some(token_id) = token_identifier
            && let Some(token_outputs) = token_outputs_state.token_outputs.get(token_id)
        {
            return Ok(Some(token_outputs.clone()));
        }
        if let Some(issuer_pk) = issuer_public_key {
            for token_outputs in token_outputs_state.token_outputs.values() {
                if &token_outputs.metadata.issuer_public_key == issuer_pk {
                    return Ok(Some(token_outputs.clone()));
                }
            }
        }
        Ok(None)
    }

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;

        match token_outputs_state
            .token_outputs
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
                token_outputs_state.token_outputs.insert(
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
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        let mut token_outputs_state = self.token_outputs.lock().await;
        let Some(token_outputs) = token_outputs_state.token_outputs.get_mut(token_identifier)
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

        if outputs.iter().map(|o| o.output.token_amount).sum::<u128>() < amount {
            return Err(TokenOutputServiceError::Generic(
                "Not enough outputs to transfer tokens".to_string(),
            ));
        }

        let selected_outputs = if let Some(output) =
            outputs.iter().find(|o| o.output.token_amount == amount)
        {
            // If there's an exact match, return it
            vec![output.clone()]
        } else {
            // TODO: support other selection strategies (JS supports either smallest or largest first)
            // Sort outputs by amount, smallest first
            outputs.sort_by_key(|o| o.output.token_amount);

            // Select outputs to match the amount
            let mut selected_outputs = Vec::new();
            let mut remaining_amount = amount;
            for output in outputs {
                if remaining_amount == 0 {
                    break;
                }
                selected_outputs.push(output.clone());
                remaining_amount = remaining_amount.saturating_sub(output.output.token_amount);
            }

            // We should never get here, but just in case
            if remaining_amount > 0 {
                return Err(TokenOutputServiceError::Generic(format!(
                    "Not enough outputs to transfer tokens, remaining amount: {remaining_amount}"
                )));
            }

            selected_outputs
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
            .collect::<Vec<_>>();
        token_outputs
            .outputs
            .retain(|to| !selected_output_ids.contains(&to.output.id));

        // Insert the reservation
        token_outputs_state
            .reservations
            .insert(reservation_id.clone(), reservation_token_outputs.clone());

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
                .token_outputs
                .get_mut(&reserved_token_outputs.metadata.identifier)
        {
            for output in reserved_token_outputs.outputs {
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
