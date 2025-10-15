use std::time::UNIX_EPOCH;

use spark_wallet::SparkWallet;

use crate::{
    Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError, TokenMetadata,
    persist::ObjectCacheRepository,
};

/// Returns the metadata for the given token identifiers.
///
/// Results are not guaranteed to be in the same order as the input token identifiers.
///
/// If the metadata is not found in the object cache, it will be queried from the Spark network.
/// The metadata is then cached in the object cache.
pub async fn get_tokens_metadata_cached_or_query(
    spark_wallet: &SparkWallet,
    object_repository: &ObjectCacheRepository,
    token_identifiers: &[&str],
) -> Result<Vec<TokenMetadata>, SdkError> {
    let mut cached_results = Vec::new();
    let mut uncached_identifiers = Vec::new();
    for token_identifier in token_identifiers {
        if let Some(metadata) = object_repository
            .fetch_token_metadata(token_identifier)
            .await?
        {
            cached_results.push(metadata);
        } else {
            uncached_identifiers.push(*token_identifier);
        }
    }

    let queried_results = spark_wallet
        .get_tokens_metadata(uncached_identifiers.as_slice())
        .await?
        .into_iter()
        .map(Into::into)
        .collect();

    for result in &queried_results {
        object_repository.save_token_metadata(result).await?;
    }

    Ok([cached_results, queried_results].concat())
}

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

    let metadata =
        get_tokens_metadata_cached_or_query(spark_wallet, object_repository, &[token_identifier])
            .await?
            .first()
            .cloned()
            .ok_or(SdkError::Generic("Token metadata not found".to_string()))?;

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
