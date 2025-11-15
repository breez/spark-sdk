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
        if amount == 0 {
            return Err(TokenOutputServiceError::Generic(
                "Amount to reserve must be greater than zero".to_string(),
            ));
        }

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

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;
    use crate::token::{TokenMetadata, TokenOutput};
    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_public_key(fill_byte: u8) -> PublicKey {
        let mut pk_bytes = [fill_byte; 33];
        pk_bytes[0] = 2; // Compressed public key prefix
        PublicKey::from_slice(&pk_bytes).unwrap()
    }

    fn create_token_outputs(identifier_no: u8, output_amounts: Vec<u128>) -> TokenOutputs {
        let identifier = format!("token-{}", identifier_no);
        let ticker = format!("TK{}", identifier_no);

        let issuer_pk = create_public_key(identifier_no);
        let owner_pk = PublicKey::from_slice(&[
            3, 23, 183, 225, 206, 31, 159, 148, 195, 42, 67, 115, 146, 41, 248, 140, 11, 3, 51, 41,
            111, 180, 110, 143, 114, 134, 88, 73, 198, 174, 52, 184, 78,
        ])
        .unwrap();

        let metadata = TokenMetadata {
            identifier: identifier.clone(),
            issuer_public_key: issuer_pk,
            name: format!("{} Token", ticker),
            ticker: ticker.to_string(),
            decimals: 8,
            max_supply: 1_000_000,
            is_freezable: false,
            creation_entity_public_key: None,
        };

        let outputs = output_amounts
            .into_iter()
            .enumerate()
            .map(|(i, amount)| TokenOutputWithPrevOut {
                output: TokenOutput {
                    id: format!("output-{}-{}", identifier, amount),
                    owner_public_key: owner_pk,
                    revocation_commitment: format!("commitment-{}", i),
                    withdraw_bond_sats: 1000,
                    withdraw_relative_block_locktime: 144,
                    token_public_key: Some(issuer_pk),
                    token_identifier: identifier.to_string(),
                    token_amount: amount,
                },
                prev_tx_hash: format!("tx-hash-{}", i),
                prev_tx_vout: i as u32,
            })
            .collect();

        TokenOutputs { metadata, outputs }
    }

    #[async_test_all]
    async fn test_default() {
        let state: InMemoryTokenOutputStore = InMemoryTokenOutputStore::default();
        assert!(state.token_outputs.lock().await.token_outputs.is_empty());
        assert!(state.token_outputs.lock().await.reservations.is_empty());
    }

    #[async_test_all]
    async fn test_set_tokens_outputs() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Verify the outputs were stored
        let stored_outputs = store.list_tokens_outputs().await.unwrap();
        assert_eq!(stored_outputs.len(), 2);
    }

    #[async_test_all]
    async fn test_get_token_outputs() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Verify token1
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.metadata.identifier, "token-1");
        assert_eq!(stored_token1.outputs.len(), 3);

        let pk1 = create_public_key(1);
        let stored_token1_by_pk = store
            .get_token_outputs(None, Some(&pk1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1_by_pk.metadata.identifier, "token-1");
        assert_eq!(stored_token1_by_pk.outputs.len(), 3);

        // Verify token2
        let stored_token2 = store
            .get_token_outputs(Some("token-2"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token2.metadata.identifier, "token-2");
        assert_eq!(stored_token2.outputs.len(), 2);

        let pk2 = create_public_key(2);
        let stored_token2_by_pk = store
            .get_token_outputs(None, Some(&pk2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token2_by_pk.metadata.identifier, "token-2");
        assert_eq!(stored_token2_by_pk.outputs.len(), 2);
    }

    #[async_test_all]
    async fn test_set_tokens_outputs_with_update() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Verify the outputs were stored
        let stored_outputs = store.list_tokens_outputs().await.unwrap();
        assert_eq!(stored_outputs.len(), 2);

        // Update with new token outputs (overwrite)
        let token1_updated = create_token_outputs(1, vec![150, 250]);
        let result = store
            .set_tokens_outputs(slice::from_ref(&token1_updated))
            .await;
        assert!(result.is_ok());

        // Verify token1 was updated
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);
        assert_eq!(stored_token1.outputs[0].output.token_amount, 150);
        assert_eq!(stored_token1.outputs[1].output.token_amount, 250);

        // Verify token2 is gone (not included in the update)
        let stored_outputs = store.list_tokens_outputs().await.unwrap();
        assert_eq!(stored_outputs.len(), 1);
    }

    #[async_test_all]
    async fn test_insert_token_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300]);

        // Set the token outputs
        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Insert outputs for a new token
        let token2 = create_token_outputs(2, vec![500, 1000]);
        let result = store.insert_token_outputs(&token2).await;
        assert!(result.is_ok());

        // Verify there are now two tokens
        let stored_outputs = store.list_tokens_outputs().await.unwrap();
        assert_eq!(stored_outputs.len(), 2);

        // Insert additional outputs for token1
        let token1_additional = create_token_outputs(1, vec![400, 500]);
        let result = store.insert_token_outputs(&token1_additional).await;
        assert!(result.is_ok());

        // Verify token1 now has 5 outputs
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 5);

        // Insert some duplicate outputs for token2 (should not duplicate)
        let token2_duplicate = create_token_outputs(2, vec![500, 750, 1000]);
        let result = store.insert_token_outputs(&token2_duplicate).await;
        assert!(result.is_ok());

        // Verify token2 now has 3 unique outputs
        let stored_token2 = store
            .get_token_outputs(Some("token-2"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token2.outputs.len(), 3);
    }

    #[async_test_all]
    async fn test_reserve_token_outputs() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();
        assert_eq!(reservation.token_outputs.metadata.identifier, "token-1");
        assert_eq!(reservation.token_outputs.outputs.len(), 1);

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);
    }

    #[async_test_all]
    async fn test_reserve_token_outputs_and_cancel() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);

        // Cancel the reservation
        let result = store.cancel_reservation(&reservation.id).await;
        assert!(result.is_ok());

        // Verify token1 has all 3 outputs back
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 3);
    }

    #[async_test_all]
    async fn test_reserve_token_outputs_and_finalize() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);

        // Finalize the reservation
        let result = store.finalize_reservation(&reservation.id).await;
        assert!(result.is_ok());

        // Verify token1 still has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);
    }

    #[async_test_all]
    async fn test_reserve_token_outputs_and_set_add_output() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);

        // Set new token outputs, simulating an external update
        let token1_updated = create_token_outputs(1, vec![100, 200, 300, 400]);
        let result = store
            .set_tokens_outputs(slice::from_ref(&token1_updated))
            .await;
        assert!(result.is_ok());

        // Verify that the reservation is reconciled and still valid
        let token_outputs_state = store.token_outputs.lock().await;
        let reserved_token_outputs = token_outputs_state
            .reservations
            .get(&reservation.id)
            .unwrap();
        assert_eq!(reserved_token_outputs.outputs.len(), 1);
        drop(token_outputs_state);

        // Verify token1 has 3 outputs (400 added, 300 reserved)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 3);
    }

    #[async_test_all]
    async fn test_reserve_token_outputs_and_set_remove_reserved_output() {
        let store = InMemoryTokenOutputStore::default();

        // Create some token outputs
        let token1 = create_token_outputs(1, vec![100, 200, 300]);
        let token2 = create_token_outputs(2, vec![500, 1000]);

        // Set the token outputs
        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);

        // Set new token outputs, simulating an external update
        let token1_updated = create_token_outputs(1, vec![100, 200, 400]);
        let result = store
            .set_tokens_outputs(slice::from_ref(&token1_updated))
            .await;
        assert!(result.is_ok());

        // Verify that the reservation is reconciled and reservation removed
        let token_outputs_state = store.token_outputs.lock().await;
        let reserved_token_outputs = token_outputs_state.reservations.get(&reservation.id);
        assert!(reserved_token_outputs.is_none());
        drop(token_outputs_state);

        // Verify token1 has 3 outputs (400 added, 300 removed)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 3);

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Verify token1 now has 1 output left
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 1);

        // Set new token outputs, simulating an external update
        let token1_updated = create_token_outputs(1, vec![100, 400]);
        let result = store
            .set_tokens_outputs(slice::from_ref(&token1_updated))
            .await;
        assert!(result.is_ok());

        // Verify that the reservation is reconciled and output removed
        let token_outputs_state = store.token_outputs.lock().await;
        let reserved_token_outputs = token_outputs_state
            .reservations
            .get(&reservation.id)
            .unwrap();
        assert_eq!(reserved_token_outputs.outputs.len(), 1);
        drop(token_outputs_state);

        // Verify token1 has 1 output (100 reserved, 200 removed, 400)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 1);
    }

    #[async_test_all]
    async fn test_multiple_parallel_reservations() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with multiple amounts
        let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

        // Set the token outputs
        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Create multiple reservations in parallel
        let reservation1 = store
            .reserve_token_outputs("token-1", 100, None)
            .await
            .unwrap();
        let reservation2 = store
            .reserve_token_outputs("token-1", 200, None)
            .await
            .unwrap();
        let reservation3 = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Verify each reservation has correct outputs
        assert_eq!(reservation1.token_outputs.outputs.len(), 1);
        assert_eq!(
            reservation1.token_outputs.outputs[0].output.token_amount,
            100
        );

        assert_eq!(reservation2.token_outputs.outputs.len(), 1);
        assert_eq!(
            reservation2.token_outputs.outputs[0].output.token_amount,
            200
        );

        assert_eq!(reservation3.token_outputs.outputs.len(), 1);
        assert_eq!(
            reservation3.token_outputs.outputs[0].output.token_amount,
            300
        );

        // Verify only 2 outputs remain available (400, 500)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 2);

        // Cancel one reservation
        let result = store.cancel_reservation(&reservation2.id).await;
        assert!(result.is_ok());

        // Verify 3 outputs are now available (200, 400, 500)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 3);

        // Finalize another reservation
        let result = store.finalize_reservation(&reservation1.id).await;
        assert!(result.is_ok());

        // Verify still 3 outputs available (finalize doesn't return outputs)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 3);

        // Cancel the last reservation
        let result = store.cancel_reservation(&reservation3.id).await;
        assert!(result.is_ok());

        // Verify 4 outputs are now available (200, 300, 400, 500)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 4);
    }

    #[async_test_all]
    async fn test_reserve_with_preferred_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Get specific outputs to use as preferred
        let all_outputs = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();

        let preferred = vec![
            all_outputs.outputs[2].clone(), // 300
            all_outputs.outputs[4].clone(), // 500
        ];

        // Reserve using preferred outputs
        let reservation = store
            .reserve_token_outputs("token-1", 250, Some(preferred))
            .await
            .unwrap();

        // Should select the exact match (300) from preferred outputs
        assert_eq!(reservation.token_outputs.outputs.len(), 1);
        assert_eq!(
            reservation.token_outputs.outputs[0].output.token_amount,
            300
        );

        // Verify 4 outputs remain (100, 200, 400, 500)
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 4);
    }

    #[async_test_all]
    async fn test_reserve_insufficient_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Try to reserve more than available
        let result = store.reserve_token_outputs("token-1", 500, None).await;
        assert!(result.is_err());

        if let Err(TokenOutputServiceError::Generic(msg)) = result {
            assert!(msg.contains("Not enough outputs"));
        }
    }

    #[async_test_all]
    async fn test_reserve_nonexistent_token() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Try to reserve from non-existent token
        let result = store.reserve_token_outputs("token-999", 100, None).await;
        assert!(result.is_err());

        if let Err(TokenOutputServiceError::Generic(msg)) = result {
            assert!(msg.contains("Token outputs not found"));
        }
    }

    #[async_test_all]
    async fn test_reserve_exact_amount_match() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![50, 100, 150, 200, 250]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve exact match amount
        let reservation = store
            .reserve_token_outputs("token-1", 150, None)
            .await
            .unwrap();

        // Should select exactly the 150 output
        assert_eq!(reservation.token_outputs.outputs.len(), 1);
        assert_eq!(
            reservation.token_outputs.outputs[0].output.token_amount,
            150
        );

        // Verify 4 outputs remain
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 4);
    }

    #[async_test_all]
    async fn test_reserve_multiple_outputs_combination() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![10, 20, 30, 40, 50]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve amount that requires combining multiple outputs
        let reservation = store
            .reserve_token_outputs("token-1", 75, None)
            .await
            .unwrap();

        // Should select smallest first: 10 + 20 + 30 + 40 = 100 >= 75
        assert!(reservation.token_outputs.outputs.len() >= 2);
        let total: u128 = reservation
            .token_outputs
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .sum();
        assert!(total >= 75);
    }

    #[async_test_all]
    async fn test_reserve_all_available_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve total amount
        let reservation = store
            .reserve_token_outputs("token-1", 600, None)
            .await
            .unwrap();

        assert_eq!(reservation.token_outputs.outputs.len(), 3);

        // Verify no outputs remain
        let stored_token1 = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored_token1.outputs.len(), 0);
    }

    #[async_test_all]
    async fn test_reserve_with_preferred_outputs_insufficient() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        let all_outputs = store
            .get_token_outputs(Some("token-1"), None)
            .await
            .unwrap()
            .unwrap();

        let preferred = vec![
            all_outputs.outputs[0].clone(), // 100
            all_outputs.outputs[1].clone(), // 200
        ];

        // Try to reserve more than preferred outputs can provide
        let result = store
            .reserve_token_outputs("token-1", 500, Some(preferred))
            .await;

        assert!(result.is_err());
    }

    #[async_test_all]
    async fn test_reserve_zero_amount() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve zero amount
        let reservation = store.reserve_token_outputs("token-1", 0, None).await;
        assert!(reservation.is_err());
    }

    #[async_test_all]
    async fn test_cancel_nonexistent_reservation() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Cancel non-existent reservation (should not error)
        let result = store
            .cancel_reservation(&"nonexistent-id".to_string())
            .await;
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_finalize_nonexistent_reservation() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Finalize non-existent reservation (should not error but warn)
        let result = store
            .finalize_reservation(&"nonexistent-id".to_string())
            .await;
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_set_removes_all_tokens() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);
        let token2 = create_token_outputs(2, vec![300, 400]);

        let result = store
            .set_tokens_outputs(&[token1.clone(), token2.clone()])
            .await;
        assert!(result.is_ok());

        // Set empty list
        let result = store.set_tokens_outputs(&[]).await;
        assert!(result.is_ok());

        // Verify all tokens are gone
        let stored_outputs = store.list_tokens_outputs().await.unwrap();
        assert_eq!(stored_outputs.len(), 0);
    }

    #[async_test_all]
    async fn test_reserve_single_large_output() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![10, 20, 1000]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve amount that's less than the large output
        let reservation = store
            .reserve_token_outputs("token-1", 500, None)
            .await
            .unwrap();

        // Should select smallest outputs first: 10 + 20 + 1000 = 1030 >= 500
        assert!(!reservation.token_outputs.outputs.is_empty());
    }

    #[async_test_all]
    async fn test_get_token_outputs_none_found() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Try to get non-existent token by identifier
        let result = store
            .get_token_outputs(Some("token-999"), None)
            .await
            .unwrap();
        assert!(result.is_none());

        // Try to get non-existent token by public key
        let pk = create_public_key(99);
        let result = store.get_token_outputs(None, Some(&pk)).await.unwrap();
        assert!(result.is_none());
    }

    #[async_test_all]
    async fn test_set_reconciles_reservation_with_empty_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve some outputs
        let _reservation = store
            .reserve_token_outputs("token-1", 300, None)
            .await
            .unwrap();

        // Set token outputs to empty list (all outputs removed)
        let result = store.set_tokens_outputs(&[]).await;
        assert!(result.is_ok());

        // Verify reservation is removed
        let token_outputs_state = store.token_outputs.lock().await;
        assert!(token_outputs_state.reservations.is_empty());
    }
}
