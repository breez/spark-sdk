    use std::slice;

    use super::*;
    use crate::token::{TokenMetadata, TokenOutput};
    use bitcoin::secp256k1::PublicKey;
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
        assert!(
            state
                .token_outputs
                .lock()
                .await
                .available_token_outputs
                .is_empty()
        );
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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.metadata.identifier, "token-1");
        assert_eq!(stored_token1.available.len(), 3);

        let pk1 = create_public_key(1);
        let stored_token1_by_pk = store
            .get_token_outputs(GetTokenOutputsFilter::IssuerPublicKey(&pk1))
            .await
            .unwrap();
        assert_eq!(stored_token1_by_pk.metadata.identifier, "token-1");
        assert_eq!(stored_token1_by_pk.available.len(), 3);

        // Verify token2
        let stored_token2 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-2"))
            .await
            .unwrap();
        assert_eq!(stored_token2.metadata.identifier, "token-2");
        assert_eq!(stored_token2.available.len(), 2);

        let pk2 = create_public_key(2);
        let stored_token2_by_pk = store
            .get_token_outputs(GetTokenOutputsFilter::IssuerPublicKey(&pk2))
            .await
            .unwrap();
        assert_eq!(stored_token2_by_pk.metadata.identifier, "token-2");
        assert_eq!(stored_token2_by_pk.available.len(), 2);
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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.available[0].output.token_amount, 150);
        assert_eq!(stored_token1.available[1].output.token_amount, 250);

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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 5);

        // Insert some duplicate outputs for token2 (should not duplicate)
        let token2_duplicate = create_token_outputs(2, vec![500, 750, 1000]);
        let result = store.insert_token_outputs(&token2_duplicate).await;
        assert!(result.is_ok());

        // Verify token2 now has 3 unique outputs
        let stored_token2 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-2"))
            .await
            .unwrap();
        assert_eq!(stored_token2.available.len(), 3);
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(reservation.token_outputs.metadata.identifier, "token-1");
        assert_eq!(reservation.token_outputs.outputs.len(), 1);

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);

        // Cancel the reservation
        let result = store.cancel_reservation(&reservation.id).await;
        assert!(result.is_ok());

        // Verify token1 has all 3 outputs back
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 3);
        assert_eq!(stored_token1.reserved_for_payment.len(), 0);
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);

        // Finalize the reservation
        let result = store.finalize_reservation(&reservation.id).await;
        assert!(result.is_ok());

        // Verify token1 still has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 0);
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);

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
        assert_eq!(reserved_token_outputs.token_outputs.outputs.len(), 1);
        drop(token_outputs_state);

        // Verify token1 has 3 outputs (400 added, 300 reserved)
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 3);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Verify token1 now has 2 outputs left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);

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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 3);
        assert_eq!(stored_token1.reserved_for_payment.len(), 0);

        // Reserve some outputs from token1
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Verify token1 now has 1 output left
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 1);
        assert_eq!(stored_token1.reserved_for_payment.len(), 2);

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
        assert_eq!(reserved_token_outputs.token_outputs.outputs.len(), 1);
        drop(token_outputs_state);

        // Verify token1 has 1 output (100 reserved, 200 removed, 400)
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 1);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(100),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();
        let reservation2 = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(200),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();
        let reservation3 = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 2);

        // Cancel one reservation
        let result = store.cancel_reservation(&reservation2.id).await;
        assert!(result.is_ok());

        // Verify 3 outputs are now available (200, 400, 500)
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 3);

        // Finalize another reservation
        let result = store.finalize_reservation(&reservation1.id).await;
        assert!(result.is_ok());

        // Verify still 3 outputs available (finalize doesn't return outputs)
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 3);

        // Cancel the last reservation
        let result = store.cancel_reservation(&reservation3.id).await;
        assert!(result.is_ok());

        // Verify 4 outputs are now available (200, 300, 400, 500)
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 4);
    }

    #[async_test_all]
    async fn test_reserve_with_preferred_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Get specific outputs to use as preferred
        let all_outputs = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();

        let preferred = vec![
            all_outputs.available[2].clone(), // 300
            all_outputs.available[4].clone(), // 500
        ];

        // Reserve using preferred outputs
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(250),
                ReservationPurpose::Payment,
                Some(preferred),
                None,
            )
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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 4);
    }

    #[async_test_all]
    async fn test_reserve_insufficient_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Try to reserve more than available
        let result = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(500),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await;
        assert!(result.is_err());

        assert!(matches!(
            result,
            Err(TokenOutputServiceError::InsufficientFunds)
        ));
    }

    #[async_test_all]
    async fn test_reserve_nonexistent_token() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Try to reserve from non-existent token
        let result = store
            .reserve_token_outputs(
                "token-999",
                ReservationTarget::MinTotalValue(100),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await;
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(150),
                ReservationPurpose::Payment,
                None,
                None,
            )
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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 4);
    }

    #[async_test_all]
    async fn test_reserve_multiple_outputs_combination() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![10, 20, 30, 40, 50]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve amount that requires combining multiple outputs
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(75),
                ReservationPurpose::Payment,
                None,
                None,
            )
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(600),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(reservation.token_outputs.outputs.len(), 3);

        // Verify no outputs remain
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 0);
    }

    #[async_test_all]
    async fn test_reserve_with_preferred_outputs_insufficient() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        let all_outputs = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();

        let preferred = vec![
            all_outputs.available[0].clone(), // 100
            all_outputs.available[1].clone(), // 200
        ];

        // Try to reserve more than preferred outputs can provide
        let result = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(500),
                ReservationPurpose::Payment,
                Some(preferred),
                None,
            )
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
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(0),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await;
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
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(500),
                ReservationPurpose::Payment,
                None,
                None,
            )
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
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-999"))
            .await;
        assert!(result.is_err());

        // Try to get non-existent token by public key
        let pk = create_public_key(99);
        let result = store
            .get_token_outputs(GetTokenOutputsFilter::IssuerPublicKey(&pk))
            .await;
        assert!(result.is_err());
    }

    #[async_test_all]
    async fn test_set_reconciles_reservation_with_empty_outputs() {
        let store = InMemoryTokenOutputStore::default();

        let token1 = create_token_outputs(1, vec![100, 200, 300]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve some outputs
        let _reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Set token outputs to empty list (all outputs removed)
        let result = store.set_tokens_outputs(&[]).await;
        assert!(result.is_ok());

        // Verify reservation is removed
        let token_outputs_state = store.token_outputs.lock().await;
        assert!(token_outputs_state.reservations.is_empty());
    }

    #[async_test_all]
    async fn test_reserve_token_outputs_selection_strategy_smallest_first() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with various amounts: [50, 100, 150, 200, 500]
        let token1 = create_token_outputs(1, vec![50, 100, 150, 200, 500]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve 300 using SmallestFirst strategy
        // Expected: 50 + 100 + 150 = 300 (smallest outputs first)
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                Some(SelectionStrategy::SmallestFirst),
            )
            .await
            .unwrap();

        // Verify selected outputs: should be 3 outputs with amounts 50, 100, 150
        assert_eq!(reservation.token_outputs.outputs.len(), 3);
        let selected_amounts: Vec<u128> = reservation
            .token_outputs
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(selected_amounts, vec![50, 100, 150]);

        // Verify total amount
        let total_selected: u128 = selected_amounts.iter().sum();
        assert_eq!(total_selected, 300);

        // Verify remaining outputs: should be [200, 500]
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let remaining_amounts: Vec<u128> = stored_token1
            .available
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(remaining_amounts, vec![200, 500]);
    }

    #[async_test_all]
    async fn test_reserve_token_outputs_selection_strategy_largest_first() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with various amounts: [50, 100, 150, 200, 500]
        let token1 = create_token_outputs(1, vec![50, 100, 150, 200, 500]);

        let result = store.set_tokens_outputs(slice::from_ref(&token1)).await;
        assert!(result.is_ok());

        // Reserve 300 using LargestFirst strategy
        // Expected: 500 (greedy algorithm takes largest available output >= target)
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(300),
                ReservationPurpose::Payment,
                None,
                Some(SelectionStrategy::LargestFirst),
            )
            .await
            .unwrap();

        // Verify selected outputs: should be 1 output with amount 500
        assert_eq!(reservation.token_outputs.outputs.len(), 1);
        let selected_amounts: Vec<u128> = reservation
            .token_outputs
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(selected_amounts, vec![500]);

        // Verify total amount
        let total_selected: u128 = selected_amounts.iter().sum();
        assert_eq!(total_selected, 500); // Note: 500 > 300, this is how the greedy algorithm works

        // Verify remaining outputs: should be [50, 100, 150, 200]
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let remaining_amounts: Vec<u128> = stored_token1
            .available
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(remaining_amounts, vec![50, 100, 150, 200]);
    }

    #[async_test_all]
    async fn test_reserve_max_output_count_smallest_first() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with amounts: [50, 100, 150, 200, 500]
        let token_outputs = create_token_outputs(1, vec![50, 100, 150, 200, 500]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Reserve with MaxOutputCount(2) using default strategy (SmallestFirst)
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MaxOutputCount(2),
                ReservationPurpose::Payment,
                None,
                None, // Default to SmallestFirst
            )
            .await
            .unwrap();

        // Verify selected outputs: should be 2 smallest outputs [50, 100]
        assert_eq!(reservation.token_outputs.outputs.len(), 2);
        let selected_amounts: Vec<u128> = reservation
            .token_outputs
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(selected_amounts, vec![50, 100]);

        // Verify remaining outputs: should be [150, 200, 500]
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let remaining_amounts: Vec<u128> = stored_token1
            .available
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(remaining_amounts, vec![150, 200, 500]);
    }

    #[async_test_all]
    async fn test_reserve_max_output_count_largest_first() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with amounts: [50, 100, 150, 200, 500]
        let token_outputs = create_token_outputs(1, vec![50, 100, 150, 200, 500]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Reserve with MaxOutputCount(3) using LargestFirst strategy
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MaxOutputCount(3),
                ReservationPurpose::Payment,
                None,
                Some(SelectionStrategy::LargestFirst),
            )
            .await
            .unwrap();

        // Verify selected outputs: should be 3 largest outputs [500, 200, 150]
        assert_eq!(reservation.token_outputs.outputs.len(), 3);
        let selected_amounts: Vec<u128> = reservation
            .token_outputs
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(selected_amounts, vec![500, 200, 150]);

        // Verify remaining outputs: should be [50, 100]
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let remaining_amounts: Vec<u128> = stored_token1
            .available
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(remaining_amounts, vec![50, 100]);
    }

    #[async_test_all]
    async fn test_reserve_max_output_count_more_than_available() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with amounts: [50, 100, 150]
        let token_outputs = create_token_outputs(1, vec![50, 100, 150]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Reserve with MaxOutputCount(10) - more than available (3)
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MaxOutputCount(10),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Should select all available outputs
        assert_eq!(reservation.token_outputs.outputs.len(), 3);
        let selected_amounts: Vec<u128> = reservation
            .token_outputs
            .outputs
            .iter()
            .map(|o| o.output.token_amount)
            .collect();
        assert_eq!(selected_amounts, vec![50, 100, 150]); // SmallestFirst by default

        // Verify no outputs remain
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.available.len(), 0);
    }

    #[async_test_all]
    async fn test_reserve_max_output_count_zero_rejected() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs
        let token_outputs = create_token_outputs(1, vec![100, 200]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Try to reserve with count = 0 - should be rejected
        let result = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MaxOutputCount(0),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), TokenOutputServiceError::Generic(msg) if msg.contains("Count to reserve must be greater than zero"))
        );
    }

    #[async_test_all]
    async fn test_reserve_for_payment_affects_balance() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with amounts: [100, 200, 300]
        let token_outputs = create_token_outputs(1, vec![100, 200, 300]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Get initial balance
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let initial_balance = stored_token1.balance();
        assert_eq!(initial_balance, 600); // 100 + 200 + 300

        // Reserve 200 for payment
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(200),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Get balance after reservation
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let balance_after_reservation = stored_token1.balance();

        // Balance should decrease by the reserved amount (200)
        // Available: 100 + 300 = 400, Reserved for payment: 200 (excluded)
        assert_eq!(balance_after_reservation, 400);
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);
        assert_eq!(stored_token1.reserved_for_swap.len(), 0);
        assert_eq!(
            stored_token1.reserved_for_payment[0].output.token_amount,
            200
        );

        // Cancel the reservation
        store.cancel_reservation(&reservation.id).await.unwrap();

        // Balance should return to original
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.balance(), initial_balance);
    }

    #[async_test_all]
    async fn test_reserve_for_swap_does_not_affect_balance() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with amounts: [100, 200, 300]
        let token_outputs = create_token_outputs(1, vec![100, 200, 300]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Get initial balance
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let initial_balance = stored_token1.balance();
        assert_eq!(initial_balance, 600); // 100 + 200 + 300

        // Reserve 200 for swap
        let reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(200),
                ReservationPurpose::Swap,
                None,
                None,
            )
            .await
            .unwrap();

        // Get balance after reservation
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let balance_after_reservation = stored_token1.balance();

        // Balance should remain the same (swap doesn't affect balance)
        // Available: 100 + 300 = 400, Reserved for swap: 200 (included in balance)
        assert_eq!(balance_after_reservation, 600);
        assert_eq!(stored_token1.available.len(), 2);
        assert_eq!(stored_token1.reserved_for_payment.len(), 0);
        assert_eq!(stored_token1.reserved_for_swap.len(), 1);
        assert_eq!(stored_token1.reserved_for_swap[0].output.token_amount, 200);

        // Cancel the reservation
        store.cancel_reservation(&reservation.id).await.unwrap();

        // Balance should still be the same
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.balance(), initial_balance);
    }

    #[async_test_all]
    async fn test_mixed_reservation_purposes_balance() {
        let store = InMemoryTokenOutputStore::default();

        // Create token outputs with amounts: [100, 200, 300, 400, 500]
        let token_outputs = create_token_outputs(1, vec![100, 200, 300, 400, 500]);
        store.set_tokens_outputs(&[token_outputs]).await.unwrap();

        // Get initial balance
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let initial_balance = stored_token1.balance();
        assert_eq!(initial_balance, 1500); // 100 + 200 + 300 + 400 + 500

        // Reserve 100 for payment
        let payment_reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(100),
                ReservationPurpose::Payment,
                None,
                None,
            )
            .await
            .unwrap();

        // Reserve 200 for swap
        let swap_reservation = store
            .reserve_token_outputs(
                "token-1",
                ReservationTarget::MinTotalValue(200),
                ReservationPurpose::Swap,
                None,
                None,
            )
            .await
            .unwrap();

        // Get balance after both reservations
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        let balance_after_reservations = stored_token1.balance();

        // Balance should only decrease by payment reservation (100)
        // Available: 300 + 400 + 500 = 1200
        // Reserved for payment: 100 (excluded)
        // Reserved for swap: 200 (included)
        // Total balance: 1200 + 200 = 1400
        assert_eq!(balance_after_reservations, 1400);
        assert_eq!(stored_token1.available.len(), 3);
        assert_eq!(stored_token1.reserved_for_payment.len(), 1);
        assert_eq!(stored_token1.reserved_for_swap.len(), 1);

        // Cancel both reservations
        store
            .cancel_reservation(&payment_reservation.id)
            .await
            .unwrap();
        store
            .cancel_reservation(&swap_reservation.id)
            .await
            .unwrap();

        // Balance should return to original
        let stored_token1 = store
            .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
            .await
            .unwrap();
        assert_eq!(stored_token1.balance(), initial_balance);
        assert_eq!(stored_token1.available.len(), 5);
        assert_eq!(stored_token1.reserved_for_payment.len(), 0);
        assert_eq!(stored_token1.reserved_for_swap.len(), 0);
    }
