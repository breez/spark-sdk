use std::sync::Arc;

use breez_sdk_common::input::{InputType, PaymentRequestSource, parse_spark_address};
use platform_utils::time::UNIX_EPOCH;
use spark_wallet::{BURN_PUBLIC_KEY, PublicKey, SparkWallet};
use tracing::{debug, warn};

use crate::{
    Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError, Storage,
    TokenMetadata, TokenTransactionType, persist::ObjectCacheRepository,
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

/// Returns whether the inputs of `transaction` are owned by `identity_public_key`.
///
/// For transfer inputs, the owner is determined by looking up the spent output
/// on `parent_transaction`. For mint and create inputs the answer is always
/// `false`. Assumes all inputs of `transaction` share the same owner public key.
pub fn token_tx_inputs_are_ours(
    transaction: &spark_wallet::TokenTransaction,
    parent_transaction: Option<&spark_wallet::TokenTransaction>,
    identity_public_key: PublicKey,
) -> Result<bool, SdkError> {
    match &transaction.inputs {
        spark_wallet::TokenInputs::Transfer(token_transfer_input) => {
            let first_input = token_transfer_input
                .outputs_to_spend
                .first()
                .ok_or_else(|| SdkError::Generic("No input in token transfer input".to_string()))?;
            let parent = parent_transaction.ok_or_else(|| {
                SdkError::Generic("Parent transaction required for transfer input".to_string())
            })?;
            let output = parent
                .outputs
                .get(first_input.prev_token_tx_vout as usize)
                .ok_or_else(|| SdkError::Generic("Output not found".to_string()))?;
            Ok(output.owner_public_key == identity_public_key)
        }
        spark_wallet::TokenInputs::Mint(_) | spark_wallet::TokenInputs::Create(_) => Ok(false),
    }
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
    // Transactions with no outputs (e.g. Create) produce no payments
    let Some(first_output) = transaction.outputs.first() else {
        debug!(
            "Skipping token transaction with no outputs: hash={}, inputs={:?}",
            hex::encode(&transaction.hash),
            transaction.inputs,
        );
        return Ok(Vec::new());
    };

    // Get token metadata for the first output (assuming all outputs have the same token)
    let token_identifier = first_output.token_identifier.as_ref();

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

        let mut tx_type = match transaction.inputs {
            spark_wallet::TokenInputs::Mint(..) => TokenTransactionType::Mint,
            spark_wallet::TokenInputs::Transfer(..) => TokenTransactionType::Transfer,
            spark_wallet::TokenInputs::Create(..) => {
                return Err(SdkError::Generic(
                    "Create token transactions are not expected to have outputs".to_string(),
                ));
            }
        };

        if output.owner_public_key == PublicKey::from_slice(BURN_PUBLIC_KEY).unwrap() {
            tx_type = TokenTransactionType::Burn;
        }

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
                tx_type,
                invoice_details: invoice.map(Into::into),
                conversion_info: None,
            }),
            conversion_details: None,
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

#[cfg(test)]
mod tests {
    use platform_utils::time::SystemTime;
    use spark_wallet::{
        TokenInputs, TokenMintInput, TokenOutput, TokenOutputToSpend, TokenTransaction,
        TokenTransactionStatus, TokenTransferInput,
    };

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn pk(fill_byte: u8) -> PublicKey {
        let mut bytes = [fill_byte; 33];
        bytes[0] = 2;
        PublicKey::from_slice(&bytes).unwrap()
    }

    fn token_output(owner: PublicKey) -> TokenOutput {
        TokenOutput {
            id: "out".to_string(),
            owner_public_key: owner,
            revocation_commitment: "commitment".to_string(),
            withdraw_bond_sats: 1000,
            withdraw_relative_block_locktime: 144,
            token_public_key: None,
            token_identifier: "tk".to_string(),
            token_amount: 100,
        }
    }

    fn transfer_tx(prev_tx_hash: &str, prev_vout: u32) -> TokenTransaction {
        TokenTransaction {
            hash: "child".to_string(),
            inputs: TokenInputs::Transfer(TokenTransferInput {
                outputs_to_spend: vec![TokenOutputToSpend {
                    prev_token_tx_hash: prev_tx_hash.to_string(),
                    prev_token_tx_vout: prev_vout,
                }],
            }),
            outputs: vec![],
            status: TokenTransactionStatus::Finalized,
            created_timestamp: SystemTime::now(),
            fulfilled_invoices: vec![],
        }
    }

    fn parent_tx(outputs: Vec<TokenOutput>) -> TokenTransaction {
        TokenTransaction {
            hash: "parent".to_string(),
            inputs: TokenInputs::Mint(TokenMintInput {
                issuer_public_key: pk(7),
                token_id: None,
            }),
            outputs,
            status: TokenTransactionStatus::Finalized,
            created_timestamp: SystemTime::now(),
            fulfilled_invoices: vec![],
        }
    }

    #[macros::test_all]
    fn transfer_owned_input_returns_true() {
        let identity = pk(1);
        let parent = parent_tx(vec![token_output(identity)]);
        let tx = transfer_tx("parent", 0);
        assert!(token_tx_inputs_are_ours(&tx, Some(&parent), identity).unwrap());
    }

    #[macros::test_all]
    fn transfer_unowned_input_returns_false() {
        let identity = pk(1);
        let other = pk(2);
        let parent = parent_tx(vec![token_output(other)]);
        let tx = transfer_tx("parent", 0);
        assert!(!token_tx_inputs_are_ours(&tx, Some(&parent), identity).unwrap());
    }

    #[macros::test_all]
    fn transfer_picks_input_at_specified_vout() {
        let identity = pk(1);
        let other = pk(2);
        let parent = parent_tx(vec![token_output(other), token_output(identity)]);
        let tx = transfer_tx("parent", 1);
        assert!(token_tx_inputs_are_ours(&tx, Some(&parent), identity).unwrap());
    }

    #[macros::test_all]
    fn transfer_missing_parent_errors() {
        let identity = pk(1);
        let tx = transfer_tx("parent", 0);
        assert!(token_tx_inputs_are_ours(&tx, None, identity).is_err());
    }

    #[macros::test_all]
    fn transfer_no_inputs_errors() {
        let identity = pk(1);
        let tx = TokenTransaction {
            hash: "child".to_string(),
            inputs: TokenInputs::Transfer(TokenTransferInput {
                outputs_to_spend: vec![],
            }),
            outputs: vec![],
            status: TokenTransactionStatus::Finalized,
            created_timestamp: SystemTime::now(),
            fulfilled_invoices: vec![],
        };
        let parent = parent_tx(vec![token_output(identity)]);
        assert!(token_tx_inputs_are_ours(&tx, Some(&parent), identity).is_err());
    }

    #[macros::test_all]
    fn transfer_vout_out_of_range_errors() {
        let identity = pk(1);
        let parent = parent_tx(vec![token_output(identity)]);
        let tx = transfer_tx("parent", 5);
        assert!(token_tx_inputs_are_ours(&tx, Some(&parent), identity).is_err());
    }

    #[macros::test_all]
    fn mint_returns_false_even_when_outputs_are_ours() {
        let identity = pk(1);
        let tx = TokenTransaction {
            hash: "mint".to_string(),
            inputs: TokenInputs::Mint(TokenMintInput {
                issuer_public_key: identity,
                token_id: None,
            }),
            outputs: vec![token_output(identity)],
            status: TokenTransactionStatus::Finalized,
            created_timestamp: SystemTime::now(),
            fulfilled_invoices: vec![],
        };
        assert!(!token_tx_inputs_are_ours(&tx, None, identity).unwrap());
    }
}
