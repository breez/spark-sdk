use std::{collections::HashMap, sync::Arc};

use breez_sdk_common::input::{
    InputType, PaymentRequestSource, SparkInvoiceDetails, parse_spark_address,
};
use platform_utils::time::UNIX_EPOCH;
use spark_wallet::{BURN_PUBLIC_KEY, PublicKey, SparkWallet};
use tracing::debug;

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
/// Assumes all inputs of a token transaction share the same owner public key.
#[allow(clippy::too_many_lines)]
pub async fn token_transaction_to_payments(
    spark_wallet: &SparkWallet,
    object_repository: &ObjectCacheRepository,
    transaction: &spark_wallet::TokenTransaction,
    tx_inputs_are_ours: bool,
) -> Result<Vec<Payment>, SdkError> {
    // Transactions with no outputs (e.g. Create) produce no payments
    if transaction.outputs.is_empty() {
        debug!(
            "Skipping token transaction with no outputs: hash={}, inputs={:?}",
            hex::encode(&transaction.hash),
            transaction.inputs,
        );
        return Ok(Vec::new());
    }

    let mut token_identifiers: Vec<&str> = transaction
        .outputs
        .iter()
        .map(|o| o.token_identifier.as_ref())
        .collect();
    token_identifiers.sort_unstable();
    token_identifiers.dedup();

    let metadata_by_token: HashMap<String, TokenMetadata> =
        get_tokens_metadata_cached_or_query(spark_wallet, object_repository, &token_identifiers)
            .await?
            .into_iter()
            .map(|m| (m.identifier.clone(), m))
            .collect();

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

    let mut unmatched_invoices = Vec::new();
    for invoice_str in &transaction.fulfilled_invoices {
        if let Some(InputType::SparkInvoice(invoice)) =
            parse_spark_address(invoice_str, &PaymentRequestSource::default())
        {
            unmatched_invoices.push(invoice);
        }
    }
    // `fulfilled_invoices` arrives in the caller's order when we broadcast the
    // transaction ourselves, and in the operators' order when we learn of it by
    // syncing. Sort by the invoice string, the order the protocol itself puts
    // attachments in, so that every wallet attributes the same invoice to the
    // same payment regardless of how it saw the transaction.
    unmatched_invoices.sort_by(|a, b| a.invoice.cmp(&b.invoice));

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

        let metadata = metadata_by_token
            .get(&output.token_identifier)
            .ok_or_else(|| {
                SdkError::Generic(format!(
                    "Token metadata not found for {}",
                    output.token_identifier
                ))
            })?;

        let invoice = take_invoice_for_output(
            &mut unmatched_invoices,
            &output.owner_public_key.to_string(),
            &output.token_identifier,
            output.token_amount,
        );

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

    if !unmatched_invoices.is_empty() {
        debug!(
            "{} fulfilled invoice(s) on tx {} matched no output",
            unmatched_invoices.len(),
            transaction.hash
        );
    }

    Ok(payments)
}

/// Picks the invoice that `output` pays, removing it so no invoice is attached to
/// two payments.
///
/// The protocol carries no output-to-invoice mapping: `InvoiceAttachment` holds
/// only the invoice string, and attachments are sorted before broadcast, so even
/// their order says nothing about which output pays which. The match is therefore
/// inferred from payee, token and amount, preferring an exact amount over an
/// invoice that left the amount to the sender.
///
/// Two invoices from the same payee for the same token and amount are
/// indistinguishable, so either assignment is equally correct. Which one is taken
/// depends on the order of `unmatched`, so callers must put it in a canonical
/// order first, or two wallets seeing the same transaction from different sources
/// would attribute those invoices differently.
fn take_invoice_for_output(
    unmatched: &mut Vec<SparkInvoiceDetails>,
    owner_public_key: &str,
    token_identifier: &str,
    amount: u128,
) -> Option<SparkInvoiceDetails> {
    let payable = |invoice: &SparkInvoiceDetails| {
        invoice.identity_public_key == owner_public_key
            && invoice.token_identifier.as_deref() == Some(token_identifier)
    };

    let position = unmatched
        .iter()
        .position(|i| payable(i) && i.amount == Some(amount))
        .or_else(|| {
            unmatched
                .iter()
                .position(|i| payable(i) && i.amount.is_none())
        })?;
    Some(unmatched.remove(position))
}

/// Persists every payment the transaction produced, one per output paying
/// someone else, in vout order.
pub(crate) async fn map_and_persist_token_transaction_payments(
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    token_transaction: &spark_wallet::TokenTransaction,
) -> Result<Vec<Payment>, SdkError> {
    let object_repository = ObjectCacheRepository::new(storage.clone());
    let payments =
        token_transaction_to_payments(spark_wallet, &object_repository, token_transaction, true)
            .await?;
    for payment in &payments {
        storage.apply_payment_update(payment.clone()).await?;
    }

    if payments.is_empty() {
        return Err(SdkError::Generic(
            "No payment created from token transaction".to_string(),
        ));
    }
    Ok(payments)
}

pub(crate) async fn map_and_persist_token_transaction(
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    token_transaction: &spark_wallet::TokenTransaction,
) -> Result<Payment, SdkError> {
    let payments =
        map_and_persist_token_transaction_payments(spark_wallet, storage, token_transaction)
            .await?;
    payments.into_iter().next().ok_or(SdkError::Generic(
        "No payment created from token transaction".to_string(),
    ))
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
            owner_public_key: owner,
            revocation_commitment: "commitment".to_string(),
            withdraw_bond_sats: 1000,
            withdraw_relative_block_locktime: 144,
            token_public_key: None,
            token_identifier: "tk".to_string(),
            token_amount: 100,
        }
    }

    const TOKEN: &str = "tk";
    const OTHER_TOKEN: &str = "tk-other";

    fn invoice(payee: &str, token: Option<&str>, amount: Option<u128>) -> SparkInvoiceDetails {
        SparkInvoiceDetails {
            invoice: format!("inv-{payee}-{token:?}-{amount:?}"),
            identity_public_key: payee.to_string(),
            network: breez_sdk_common::network::BitcoinNetwork::Regtest,
            amount,
            token_identifier: token.map(ToString::to_string),
            expiry_time: None,
            description: None,
            sender_public_key: None,
        }
    }

    #[macros::test_all]
    fn take_invoice_matches_same_payee_by_amount() {
        // Two invoices from one payee: each output must take its own.
        let mut unmatched = vec![
            invoice("alice", Some(TOKEN), Some(100)),
            invoice("alice", Some(TOKEN), Some(250)),
        ];

        let first = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 250).unwrap();
        assert_eq!(first.amount, Some(250));
        let second = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).unwrap();
        assert_eq!(second.amount, Some(100));
        assert!(unmatched.is_empty());
    }

    #[macros::test_all]
    fn take_invoice_consumes_indistinguishable_invoices_exactly_once() {
        // Same payee, token and amount: either assignment is correct, but each
        // invoice must be attached once and none dropped. Real invoices always
        // differ by their id, so give the fixtures distinct strings.
        let mut first_invoice = invoice("alice", Some(TOKEN), Some(100));
        first_invoice.invoice = "inv-one".to_string();
        let mut second_invoice = invoice("alice", Some(TOKEN), Some(100));
        second_invoice.invoice = "inv-two".to_string();
        let mut unmatched = vec![first_invoice, second_invoice];

        let first = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).unwrap();
        let second = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).unwrap();
        assert_ne!(
            first.invoice, second.invoice,
            "the same invoice was attached to two payments"
        );
        assert!(unmatched.is_empty(), "no invoice left unattached");
    }

    #[macros::test_all]
    fn take_invoice_prefers_exact_amount_over_amountless() {
        let mut unmatched = vec![
            invoice("alice", Some(TOKEN), None),
            invoice("alice", Some(TOKEN), Some(100)),
        ];

        let exact = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).unwrap();
        assert_eq!(exact.amount, Some(100));
        // The amountless one is still available for the next output.
        let amountless = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 77).unwrap();
        assert_eq!(amountless.amount, None);
    }

    #[macros::test_all]
    fn take_invoice_falls_back_to_amountless() {
        let mut unmatched = vec![invoice("alice", Some(TOKEN), None)];
        let matched = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 4321).unwrap();
        assert_eq!(matched.amount, None);
    }

    #[macros::test_all]
    fn take_invoice_requires_matching_payee() {
        let mut unmatched = vec![invoice("alice", Some(TOKEN), Some(100))];
        assert!(take_invoice_for_output(&mut unmatched, "bob", TOKEN, 100).is_none());
        assert_eq!(unmatched.len(), 1, "a non-match must not consume");
    }

    #[macros::test_all]
    fn take_invoice_requires_matching_token() {
        let mut unmatched = vec![invoice("alice", Some(TOKEN), Some(100))];
        assert!(take_invoice_for_output(&mut unmatched, "alice", OTHER_TOKEN, 100).is_none());
        assert_eq!(unmatched.len(), 1);
    }

    #[macros::test_all]
    fn take_invoice_ignores_sats_invoices() {
        let mut unmatched = vec![invoice("alice", None, Some(100))];
        assert!(take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).is_none());
    }

    #[macros::test_all]
    fn attribution_is_independent_of_the_order_invoices_arrive_in() {
        // Same payee, token and amount, so only ordering can decide the match.
        // The sending wallet sees them in its caller's order, a syncing wallet in
        // the operators' order; both must reach the same attribution.
        let mut a = invoice("alice", Some(TOKEN), Some(100));
        a.invoice = "inv-aaa".to_string();
        let mut b = invoice("alice", Some(TOKEN), Some(100));
        b.invoice = "inv-bbb".to_string();

        let attribute = |mut invoices: Vec<SparkInvoiceDetails>| {
            // Mirrors token_transaction_to_payments: canonicalise, then match each
            // output in vout order.
            invoices.sort_by(|x, y| x.invoice.cmp(&y.invoice));
            (0..2)
                .map(|_| {
                    take_invoice_for_output(&mut invoices, "alice", TOKEN, 100)
                        .map(|i| i.invoice)
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            attribute(vec![a.clone(), b.clone()]),
            attribute(vec![b, a]),
            "attribution changed with the order the invoices arrived in"
        );
    }

    #[macros::test_all]
    fn take_invoice_on_empty_list_is_none() {
        let mut unmatched = Vec::new();
        assert!(take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).is_none());
    }

    #[macros::test_all]
    fn take_invoice_separates_payees_sharing_an_amount() {
        // A mixed batch: same amount, different payees.
        let mut unmatched = vec![
            invoice("alice", Some(TOKEN), Some(100)),
            invoice("bob", Some(TOKEN), Some(100)),
        ];

        let for_bob = take_invoice_for_output(&mut unmatched, "bob", TOKEN, 100).unwrap();
        assert_eq!(for_bob.identity_public_key, "bob");
        let for_alice = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).unwrap();
        assert_eq!(for_alice.identity_public_key, "alice");
    }

    #[macros::test_all]
    fn take_invoice_separates_tokens_sharing_an_amount() {
        let mut unmatched = vec![
            invoice("alice", Some(TOKEN), Some(100)),
            invoice("alice", Some(OTHER_TOKEN), Some(100)),
        ];

        let other = take_invoice_for_output(&mut unmatched, "alice", OTHER_TOKEN, 100).unwrap();
        assert_eq!(other.token_identifier.as_deref(), Some(OTHER_TOKEN));
        let first = take_invoice_for_output(&mut unmatched, "alice", TOKEN, 100).unwrap();
        assert_eq!(first.token_identifier.as_deref(), Some(TOKEN));
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
