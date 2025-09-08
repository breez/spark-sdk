use std::time::UNIX_EPOCH;

use spark_wallet::SparkWallet;

use crate::{
    Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError,
    persist::ObjectCacheRepository,
};

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
    object_repository: &ObjectCacheRepository,
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

    let metadata = if let Some(metadata) = object_repository
        .fetch_token_metadata(token_identifier)
        .await?
    {
        metadata
    } else {
        let metadata = spark_wallet
            .get_tokens_metadata(&[token_identifier])
            .await?
            .first()
            .ok_or(SdkError::Generic("Token metadata not found".to_string()))?
            .clone()
            .into();
        object_repository.save_token_metadata(&metadata).await?;
        metadata
    };

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

    let identity_public_key = spark_wallet.get_identity_public_key();

    let mut payments = Vec::new();

    for (vout, output) in transaction.outputs.iter().enumerate() {
        let payment_type = if tx_inputs_are_ours && output.owner_public_key != identity_public_key {
            // If inputs are ours and outputs are not ours, add an outgoing payment
            PaymentType::Send
        } else if !tx_inputs_are_ours && output.owner_public_key == identity_public_key {
            // If inputs are not ours and outputs are ours, add an incoming payment
            PaymentType::Receive
        } else {
            continue;
        };

        let id = format!("{}:{}", transaction.hash, vout);

        let payment = Payment {
            id,
            payment_type,
            status: PaymentStatus::from_token_transaction_status(
                transaction.status,
                is_transfer_transaction,
            ),
            amount: output.token_amount,
            fees: 0, // TODO: calculate actual fees when they start being charged
            timestamp,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: metadata.clone(),
                tx_hash: transaction.hash.clone(),
            }),
        };
        payments.push(payment);
    }

    Ok(payments)
}
