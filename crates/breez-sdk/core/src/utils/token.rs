use std::sync::Arc;

use breez_sdk_common::input::{InputType, PaymentRequestSource, parse_spark_address};
use spark_wallet::SparkWallet;
use tracing::warn;
use web_time::UNIX_EPOCH;

use crate::{
    Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError, Storage,
    TokenMetadata, persist::ObjectCacheRepository,
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
        .get_tokens_metadata(uncached_identifiers.as_slice(), &[])
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
/// Each resulting payment corresponds to a tx output (change outputs don't result in payments).
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

    let is_mint_transaction = matches!(&transaction.inputs, spark_wallet::TokenInputs::Mint(..));
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

    let mut invoices = Vec::new();
    for invoice_str in &transaction.fulfilled_invoices {
        if let Some(InputType::SparkInvoice(invoice)) =
            parse_spark_address(invoice_str, &PaymentRequestSource::default())
        {
            invoices.push(invoice);
        }
    }

    for (vout, output) in transaction.outputs.iter().enumerate() {
        let payment_type = if tx_inputs_are_ours && output.owner_public_key != identity_public_key {
            // If inputs are ours and outputs are not ours, add an outgoing payment
            PaymentType::Send
        } else if (!tx_inputs_are_ours || is_mint_transaction)
            && output.owner_public_key == identity_public_key
        {
            // Add an incoming payment if:
            // - If inputs are not ours and outputs are ours
            // - If it's a mint transaction and outputs are ours
            PaymentType::Receive
        } else {
            continue;
        };

        let id = format!("{}:{}", transaction.hash, vout);

        // TODO:The following breaks if there are multiple invoices/outputs with the same owner public key but is the best we can do for now
        // Should be an edge case given that the Spark SDK only supports one invoice per transaction
        let invoices = invoices
            .iter()
            .filter(|i| i.identity_public_key == output.owner_public_key.to_string())
            .collect::<Vec<_>>();
        if invoices.len() > 1 {
            warn!(
                "Multiple invoices found for output owner public key: {}. Using the first one",
                output.owner_public_key
            );
        }
        let invoice = invoices.first().map(|&inv| inv.clone());

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
                invoice_details: invoice.map(Into::into),
                conversion_info: None,
            }),
        };
        payments.push(payment);
    }

    Ok(payments)
}

pub(crate) async fn map_and_persist_token_transaction(
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    token_transaction: &spark_wallet::TokenTransaction,
) -> Result<Payment, SdkError> {
    let object_repository = ObjectCacheRepository::new(storage.clone());
    let payments =
        token_transaction_to_payments(spark_wallet, &object_repository, token_transaction, true)
            .await?;
    for payment in &payments {
        storage.insert_payment(payment.clone()).await?;
    }

    payments
        .first()
        .ok_or(SdkError::Generic(
            "No payment created from token invoice".to_string(),
        ))
        .cloned()
}
