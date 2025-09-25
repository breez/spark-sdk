use std::time::UNIX_EPOCH;

use spark_wallet::{SparkWallet, TokenMetadata};

use crate::{Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError};

/// Converts a token transaction to payments
///
/// Each resulting payment corresponds to a potential group of outputs that share the same owner public key.
/// The id of the payment is the id of the first output in the group.
///
/// Assumptions:
/// - All outputs of a token transaction share the same token identifier
/// - All inputs of a token transaction share the same owner public key
#[allow(clippy::too_many_lines)]
pub async fn token_transaction_to_payments(
    spark_wallet: &SparkWallet,
    transaction: &spark_wallet::TokenTransaction,
    tx_inputs_are_ours: bool,
) -> Result<Vec<Payment>, SdkError> {
    // Get token metadata for the first output (assuming all outputs have the same token)
    let token_identifier = transaction
        .outputs
        .first()
        .ok_or(SdkError::Generic(
            "No outputs in token transaction".to_string(),
        ))?
        .token_identifier
        .as_ref();
    let metadata: TokenMetadata = spark_wallet
        .get_tokens_metadata(&[token_identifier])
        .await?
        .first()
        .ok_or(SdkError::Generic("Token metadata not found".to_string()))?
        .clone();

    let is_transfer_transaction =
        matches!(&transaction.inputs, spark_wallet::TokenInputs::Transfer(..));

    let timestamp = transaction
        .created_timestamp
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            SdkError::Generic(
                "Token transaction created timestamp is before UNIX_EPOCH".to_string(),
            )
        })?
        .as_secs();

    // Group outputs by owner public key
    let mut outputs_by_owner = std::collections::HashMap::new();
    for output in &transaction.outputs {
        outputs_by_owner
            .entry(output.owner_public_key)
            .or_insert_with(Vec::new)
            .push(output);
    }

    let mut payments = Vec::new();

    if tx_inputs_are_ours {
        // If inputs are ours, add an outgoing payment for each output group that is not ours
        for (owner_pubkey, outputs) in outputs_by_owner {
            if owner_pubkey != spark_wallet.get_identity_public_key() {
                // This is an outgoing payment to another user
                let total_amount = outputs
                    .iter()
                    .map(|output| {
                        let amount: u64 = output.token_amount.try_into().unwrap_or_default();
                        amount
                    })
                    .sum();

                let id = outputs
                    .first()
                    .ok_or(SdkError::Generic("No outputs in output group".to_string()))?
                    .id
                    .clone();

                let payment = Payment {
                    id,
                    payment_type: PaymentType::Send,
                    status: PaymentStatus::from_token_transaction_status(
                        transaction.status,
                        is_transfer_transaction,
                    ),
                    amount: total_amount,
                    fees: 0, // TODO: calculate actual fees when they start being charged
                    timestamp,
                    method: PaymentMethod::Token,
                    details: Some(PaymentDetails::Token {
                        metadata: metadata.clone().into(),
                        tx_hash: transaction.hash.clone(),
                    }),
                };

                payments.push(payment);
            }
            // Ignore outputs that belong to us (potential change outputs)
        }
    } else {
        // If inputs are not ours, add an incoming payment for our output group
        if let Some(our_outputs) = outputs_by_owner.get(&spark_wallet.get_identity_public_key()) {
            let total_amount: u64 = our_outputs
                .iter()
                .map(|output| {
                    let amount: u64 = output.token_amount.try_into().unwrap_or_default();
                    amount
                })
                .sum();

            let id = our_outputs
                .first()
                .ok_or(SdkError::Generic(
                    "No outputs in our output group".to_string(),
                ))?
                .id
                .clone();

            let payment = Payment {
                id,
                payment_type: PaymentType::Receive,
                status: PaymentStatus::from_token_transaction_status(
                    transaction.status,
                    is_transfer_transaction,
                ),
                amount: total_amount,
                fees: 0,
                timestamp,
                method: PaymentMethod::Token,
                details: Some(PaymentDetails::Token {
                    metadata: metadata.into(),
                    tx_hash: transaction.hash.clone(),
                }),
            };

            payments.push(payment);
        }
        // Ignore outputs that don't belong to us
    }

    Ok(payments)
}
