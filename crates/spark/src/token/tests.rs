//! Shared test suite for `TokenOutputStore` implementations.
//!
//! Each function tests a specific behavior against any `TokenOutputStore` impl.
//! To use, call these functions from implementation-specific test modules
//! passing a concrete store instance.

use std::slice;
use std::time::Duration;

use bitcoin::secp256k1::PublicKey;
use platform_utils::time::SystemTime;

use crate::token::{
    GetTokenOutputsFilter, ReservationPurpose, ReservationTarget, SelectionStrategy, TokenMetadata,
    TokenOutput, TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
};

pub fn create_public_key(fill_byte: u8) -> PublicKey {
    let mut pk_bytes = [fill_byte; 33];
    pk_bytes[0] = 2; // Compressed public key prefix
    PublicKey::from_slice(&pk_bytes).unwrap()
}

pub fn create_token_outputs(identifier_no: u8, output_amounts: Vec<u128>) -> TokenOutputs {
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

/// Returns a future `SystemTime`, ensuring that outputs added "now" are
/// treated as old relative to this refresh start.
pub fn future_refresh_start() -> SystemTime {
    SystemTime::now() + Duration::from_secs(10)
}

pub async fn test_set_tokens_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

    let stored_outputs = store.list_tokens_outputs().await.unwrap();
    assert_eq!(stored_outputs.len(), 2);
}

pub async fn test_get_token_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
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

pub async fn test_set_tokens_outputs_with_update(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

    let stored_outputs = store.list_tokens_outputs().await.unwrap();
    assert_eq!(stored_outputs.len(), 2);

    // Update with new token outputs (overwrite)
    let token1_updated = create_token_outputs(1, vec![150, 250]);
    let result = store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // Verify token1 was updated
    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 2);
    let mut amounts: Vec<u128> = stored_token1
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    amounts.sort_unstable();
    assert_eq!(amounts, vec![150, 250]);

    // Verify token2 is gone (not included in the update)
    let stored_outputs = store.list_tokens_outputs().await.unwrap();
    assert_eq!(stored_outputs.len(), 1);
}

pub async fn test_insert_token_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
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

pub async fn test_reserve_token_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
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

pub async fn test_reserve_token_outputs_and_cancel(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

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

pub async fn test_reserve_token_outputs_and_finalize(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

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

pub async fn test_reserve_token_outputs_and_set_add_output(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 2);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);

    // Set new token outputs, simulating an external update
    let token1_updated = create_token_outputs(1, vec![100, 200, 300, 400]);
    let result = store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // Verify token1 has 3 available + 1 reserved (reservation reconciled)
    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 3);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);
}

pub async fn test_reserve_token_outputs_and_set_remove_reserved_output(
    store: &dyn TokenOutputStore,
) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    let token2 = create_token_outputs(2, vec![500, 1000]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 2);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);

    // Set new token outputs without the reserved output
    let token1_updated = create_token_outputs(1, vec![100, 200, 400]);
    let result = store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // Verify reservation removed (reserved output no longer exists)
    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 3);
    assert_eq!(stored_token1.reserved_for_payment.len(), 0);
}

pub async fn test_multiple_parallel_reservations(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    // Create multiple reservations
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

    assert_eq!(reservation1.token_outputs.outputs.len(), 1);
    assert_eq!(reservation2.token_outputs.outputs.len(), 1);
    assert_eq!(reservation3.token_outputs.outputs.len(), 1);

    // Verify only 2 outputs remain available
    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 2);

    // Cancel one reservation
    let result = store.cancel_reservation(&reservation2.id).await;
    assert!(result.is_ok());

    // Verify 3 outputs are now available
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

    // Verify 4 outputs are now available
    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 4);
}

pub async fn test_reserve_with_preferred_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    let all_outputs = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();

    let preferred = vec![
        all_outputs
            .available
            .iter()
            .find(|o| o.output.token_amount == 300)
            .unwrap()
            .clone(),
        all_outputs
            .available
            .iter()
            .find(|o| o.output.token_amount == 500)
            .unwrap()
            .clone(),
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

    // Verify 4 outputs remain
    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 4);
}

pub async fn test_reserve_insufficient_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

pub async fn test_reserve_nonexistent_token(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

pub async fn test_reserve_exact_amount_match(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![50, 100, 150, 200, 250]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    assert_eq!(reservation.token_outputs.outputs.len(), 1);
    assert_eq!(
        reservation.token_outputs.outputs[0].output.token_amount,
        150
    );

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 4);
}

pub async fn test_reserve_multiple_outputs_combination(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![10, 20, 30, 40, 50]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    assert!(reservation.token_outputs.outputs.len() >= 2);
    let total: u128 = reservation
        .token_outputs
        .outputs
        .iter()
        .map(|o| o.output.token_amount)
        .sum();
    assert!(total >= 75);
}

pub async fn test_reserve_all_available_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 0);
}

pub async fn test_reserve_with_preferred_outputs_insufficient(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300, 400, 500]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    let all_outputs = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();

    let preferred = vec![
        all_outputs
            .available
            .iter()
            .find(|o| o.output.token_amount == 100)
            .unwrap()
            .clone(),
        all_outputs
            .available
            .iter()
            .find(|o| o.output.token_amount == 200)
            .unwrap()
            .clone(),
    ];

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

pub async fn test_reserve_zero_amount(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

pub async fn test_cancel_nonexistent_reservation(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    let result = store
        .cancel_reservation(&"nonexistent-id".to_string())
        .await;
    assert!(result.is_ok());
}

pub async fn test_finalize_nonexistent_reservation(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    let result = store
        .finalize_reservation(&"nonexistent-id".to_string())
        .await;
    assert!(result.is_ok());
}

pub async fn test_set_removes_all_tokens(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);
    let token2 = create_token_outputs(2, vec![300, 400]);

    let result = store
        .set_tokens_outputs(&[token1.clone(), token2.clone()], future_refresh_start())
        .await;
    assert!(result.is_ok());

    let result = store.set_tokens_outputs(&[], future_refresh_start()).await;
    assert!(result.is_ok());

    let stored_outputs = store.list_tokens_outputs().await.unwrap();
    assert_eq!(stored_outputs.len(), 0);
}

pub async fn test_reserve_single_large_output(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![10, 20, 1000]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    assert!(!reservation.token_outputs.outputs.is_empty());
}

pub async fn test_get_token_outputs_none_found(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

    let result = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-999"))
        .await;
    assert!(result.is_err());

    let pk = create_public_key(99);
    let result = store
        .get_token_outputs(GetTokenOutputsFilter::IssuerPublicKey(&pk))
        .await;
    assert!(result.is_err());
}

pub async fn test_set_reconciles_reservation_with_empty_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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
    let result = store.set_tokens_outputs(&[], future_refresh_start()).await;
    assert!(result.is_ok());

    // Verify no outputs remain
    let stored_outputs = store.list_tokens_outputs().await.unwrap();
    assert!(stored_outputs.is_empty());
}

pub async fn test_reserve_token_outputs_selection_strategy_smallest_first(
    store: &dyn TokenOutputStore,
) {
    let token1 = create_token_outputs(1, vec![50, 100, 150, 200, 500]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    assert_eq!(reservation.token_outputs.outputs.len(), 3);
    let selected_amounts: Vec<u128> = reservation
        .token_outputs
        .outputs
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    assert_eq!(selected_amounts, vec![50, 100, 150]);

    let total_selected: u128 = selected_amounts.iter().sum();
    assert_eq!(total_selected, 300);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let mut remaining_amounts: Vec<u128> = stored_token1
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    remaining_amounts.sort_unstable();
    assert_eq!(remaining_amounts, vec![200, 500]);
}

pub async fn test_reserve_token_outputs_selection_strategy_largest_first(
    store: &dyn TokenOutputStore,
) {
    let token1 = create_token_outputs(1, vec![50, 100, 150, 200, 500]);

    let result = store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await;
    assert!(result.is_ok());

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

    assert_eq!(reservation.token_outputs.outputs.len(), 1);
    let selected_amounts: Vec<u128> = reservation
        .token_outputs
        .outputs
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    assert_eq!(selected_amounts, vec![500]);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let mut remaining_amounts: Vec<u128> = stored_token1
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    remaining_amounts.sort_unstable();
    assert_eq!(remaining_amounts, vec![50, 100, 150, 200]);
}

pub async fn test_reserve_max_output_count_smallest_first(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![50, 100, 150, 200, 500]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MaxOutputCount(2),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(reservation.token_outputs.outputs.len(), 2);
    let mut selected_amounts: Vec<u128> = reservation
        .token_outputs
        .outputs
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    selected_amounts.sort_unstable();
    assert_eq!(selected_amounts, vec![50, 100]);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let mut remaining_amounts: Vec<u128> = stored_token1
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    remaining_amounts.sort_unstable();
    assert_eq!(remaining_amounts, vec![150, 200, 500]);
}

pub async fn test_reserve_max_output_count_largest_first(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![50, 100, 150, 200, 500]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

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

    assert_eq!(reservation.token_outputs.outputs.len(), 3);
    let mut selected_amounts: Vec<u128> = reservation
        .token_outputs
        .outputs
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    selected_amounts.sort_unstable();
    assert_eq!(selected_amounts, vec![150, 200, 500]);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let mut remaining_amounts: Vec<u128> = stored_token1
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    remaining_amounts.sort_unstable();
    assert_eq!(remaining_amounts, vec![50, 100]);
}

pub async fn test_reserve_max_output_count_more_than_available(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![50, 100, 150]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

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

    assert_eq!(reservation.token_outputs.outputs.len(), 3);
    let selected_amounts: Vec<u128> = reservation
        .token_outputs
        .outputs
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    assert_eq!(selected_amounts, vec![50, 100, 150]);

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.available.len(), 0);
}

pub async fn test_reserve_max_output_count_zero_rejected(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![100, 200]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

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

pub async fn test_reserve_for_payment_affects_balance(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![100, 200, 300]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let initial_balance = stored_token1.balance();
    assert_eq!(initial_balance, 600);

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

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let balance_after_reservation = stored_token1.balance();

    assert_eq!(balance_after_reservation, 400);
    assert_eq!(stored_token1.available.len(), 2);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);
    assert_eq!(stored_token1.reserved_for_swap.len(), 0);
    assert_eq!(
        stored_token1.reserved_for_payment[0].output.token_amount,
        200
    );

    store.cancel_reservation(&reservation.id).await.unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.balance(), initial_balance);
}

pub async fn test_reserve_for_swap_does_not_affect_balance(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![100, 200, 300]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let initial_balance = stored_token1.balance();
    assert_eq!(initial_balance, 600);

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

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let balance_after_reservation = stored_token1.balance();

    assert_eq!(balance_after_reservation, 600);
    assert_eq!(stored_token1.available.len(), 2);
    assert_eq!(stored_token1.reserved_for_payment.len(), 0);
    assert_eq!(stored_token1.reserved_for_swap.len(), 1);
    assert_eq!(stored_token1.reserved_for_swap[0].output.token_amount, 200);

    store.cancel_reservation(&reservation.id).await.unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.balance(), initial_balance);
}

pub async fn test_mixed_reservation_purposes_balance(store: &dyn TokenOutputStore) {
    let token_outputs = create_token_outputs(1, vec![100, 200, 300, 400, 500]);
    store
        .set_tokens_outputs(&[token_outputs], future_refresh_start())
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let initial_balance = stored_token1.balance();
    assert_eq!(initial_balance, 1500);

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

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    let balance_after_reservations = stored_token1.balance();

    assert_eq!(balance_after_reservations, 1400);
    assert_eq!(stored_token1.available.len(), 3);
    assert_eq!(stored_token1.reserved_for_payment.len(), 1);
    assert_eq!(stored_token1.reserved_for_swap.len(), 1);

    store
        .cancel_reservation(&payment_reservation.id)
        .await
        .unwrap();
    store
        .cancel_reservation(&swap_reservation.id)
        .await
        .unwrap();

    let stored_token1 = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored_token1.balance(), initial_balance);
    assert_eq!(stored_token1.available.len(), 5);
    assert_eq!(stored_token1.reserved_for_payment.len(), 0);
    assert_eq!(stored_token1.reserved_for_swap.len(), 0);
}

// ==================== Race condition protection tests ====================

pub async fn test_set_tokens_outputs_skipped_during_active_swap(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);
    store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await
        .unwrap();

    // Reserve for swap
    let _reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(300),
            ReservationPurpose::Swap,
            None,
            None,
        )
        .await
        .unwrap();

    // Try to set new outputs while swap is active - should be skipped
    let token1_updated = create_token_outputs(1, vec![500]);
    store
        .set_tokens_outputs(slice::from_ref(&token1_updated), future_refresh_start())
        .await
        .unwrap();

    // Verify the set was skipped - swap reservation still present
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.reserved_for_swap.len(), 2);
    assert_eq!(stored.available.len(), 0);
}

pub async fn test_set_tokens_outputs_skipped_after_swap_completes_during_refresh(
    store: &dyn TokenOutputStore,
) {
    let token1 = create_token_outputs(1, vec![100, 200]);
    store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await
        .unwrap();

    // Reserve for swap
    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(300),
            ReservationPurpose::Swap,
            None,
            None,
        )
        .await
        .unwrap();

    // Refresh starts
    let refresh_start = store.now().await.unwrap();

    // Small delay to ensure swap completes after refresh started
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // Swap completes (finalize marks spent + records swap completion)
    store.finalize_reservation(&reservation.id).await.unwrap();

    // Insert new outputs (simulating swap result)
    let token1_new = create_token_outputs(1, vec![300]);
    store.insert_token_outputs(&token1_new).await.unwrap();

    // Try to set with stale data - should be skipped because swap completed during refresh
    let token1_stale = create_token_outputs(1, vec![100, 200]);
    store
        .set_tokens_outputs(slice::from_ref(&token1_stale), refresh_start)
        .await
        .unwrap();

    // Verify the swap result outputs are preserved
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 1);
    assert_eq!(stored.available[0].output.token_amount, 300);
}

pub async fn test_insert_outputs_preserved_by_set_tokens_outputs(store: &dyn TokenOutputStore) {
    // Add initial outputs
    let token1 = create_token_outputs(1, vec![100]);
    store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await
        .unwrap();

    // Refresh starts
    let refresh_start = store.now().await.unwrap();

    // Small delay to ensure the new output is added AFTER refresh_start
    platform_utils::tokio::time::sleep(Duration::from_millis(10)).await;

    // While refresh is in progress, a new output arrives
    let token1_new = create_token_outputs(1, vec![200]);
    store.insert_token_outputs(&token1_new).await.unwrap();

    // Refresh completes with stale data (doesn't include the 200 output)
    let token1_stale = create_token_outputs(1, vec![100]);
    store
        .set_tokens_outputs(slice::from_ref(&token1_stale), refresh_start)
        .await
        .unwrap();

    // The 200 output should be preserved because it was added after refresh started
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 2);
    let amounts: Vec<u128> = stored
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    assert!(amounts.contains(&100));
    assert!(
        amounts.contains(&200),
        "Output added after refresh started should be preserved"
    );
}

pub async fn test_spent_outputs_not_restored_by_set_tokens_outputs(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200]);
    store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await
        .unwrap();

    // Reserve output-token-1-100 for payment
    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(100),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();

    // Finalize (marks as spent)
    store.finalize_reservation(&reservation.id).await.unwrap();

    // Verify only 200 remains
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 1);
    assert_eq!(stored.available[0].output.token_amount, 200);

    // Simulate a stale refresh that tries to bring back the spent output
    let refresh_start = SystemTime::now() - Duration::from_secs(60);
    let token1_stale = create_token_outputs(1, vec![100, 200, 300]);
    store
        .set_tokens_outputs(slice::from_ref(&token1_stale), refresh_start)
        .await
        .unwrap();

    // The spent 100 output should NOT be restored
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 2);
    let amounts: Vec<u128> = stored
        .available
        .iter()
        .map(|o| o.output.token_amount)
        .collect();
    assert!(amounts.contains(&200));
    assert!(amounts.contains(&300));
    assert!(
        !amounts.contains(&100),
        "Spent output should not be restored by stale refresh"
    );
}

pub async fn test_finalize_swap_marks_spent_and_tracks_completion(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100, 200, 300]);
    store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await
        .unwrap();

    // Reserve for swap
    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(600),
            ReservationPurpose::Swap,
            None,
            None,
        )
        .await
        .unwrap();

    // Finalize swap
    store.finalize_reservation(&reservation.id).await.unwrap();

    // Insert new outputs (simulating swap result)
    let token1_new = create_token_outputs(1, vec![600]);
    store.insert_token_outputs(&token1_new).await.unwrap();

    // Verify only the new output exists
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 1);
    assert_eq!(stored.available[0].output.token_amount, 600);

    // A refresh that started before the swap should be skipped
    let old_refresh = SystemTime::now() - Duration::from_secs(60);
    let token1_stale = create_token_outputs(1, vec![100, 200, 300]);
    store
        .set_tokens_outputs(slice::from_ref(&token1_stale), old_refresh)
        .await
        .unwrap();

    // The 600 output should still be there (skipped due to swap completion)
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 1);
    assert_eq!(stored.available[0].output.token_amount, 600);
}

pub async fn test_insert_outputs_clears_spent_status(store: &dyn TokenOutputStore) {
    let token1 = create_token_outputs(1, vec![100]);
    store
        .set_tokens_outputs(slice::from_ref(&token1), future_refresh_start())
        .await
        .unwrap();

    // Reserve and finalize (marks as spent)
    let reservation = store
        .reserve_token_outputs(
            "token-1",
            ReservationTarget::MinTotalValue(100),
            ReservationPurpose::Payment,
            None,
            None,
        )
        .await
        .unwrap();
    store.finalize_reservation(&reservation.id).await.unwrap();

    // Insert the same output back (simulating receiving it back)
    let token1_back = create_token_outputs(1, vec![100]);
    store.insert_token_outputs(&token1_back).await.unwrap();

    // Verify it's available
    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 1);
    assert_eq!(stored.available[0].output.token_amount, 100);

    // Now a stale refresh should not remove it (spent status was cleared)
    let old_refresh = SystemTime::now() - Duration::from_secs(60);
    let token1_refresh = create_token_outputs(1, vec![100, 200]);
    store
        .set_tokens_outputs(slice::from_ref(&token1_refresh), old_refresh)
        .await
        .unwrap();

    let stored = store
        .get_token_outputs(GetTokenOutputsFilter::Identifier("token-1"))
        .await
        .unwrap();
    assert_eq!(stored.available.len(), 2);
}
