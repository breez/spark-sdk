use std::collections::HashSet;

use spark_wallet::MAX_TOKEN_TX_OUTPUTS;

use crate::{
    InputType, SparkInvoiceDetails,
    error::SdkError,
    models::{
        PrepareSendTokenBatchRequest, PrepareSendTokenBatchResponse, ResolvedTokenBatchRecipient,
        TokenBatchRecipient, TokenBatchTotal,
    },
    sdk::BreezSdk,
    sdk::payments::validation,
};

pub(in crate::sdk::payments) async fn prepare(
    sdk: &BreezSdk,
    request: PrepareSendTokenBatchRequest,
) -> Result<PrepareSendTokenBatchResponse, SdkError> {
    if request.recipients.is_empty() {
        return Err(SdkError::InvalidInput(
            "At least one recipient is required".to_string(),
        ));
    }

    let identity_public_key = sdk.spark_wallet.get_identity_public_key().to_string();

    let mut recipients = Vec::with_capacity(request.recipients.len());
    let mut invoices = HashSet::new();
    for recipient in request.recipients {
        let resolved = resolve(sdk, recipient, &identity_public_key).await?;
        // The same invoice twice would attach twice and pay twice for one
        // request. A repeated plain address is two outputs to one payee, which
        // is a legitimate batch.
        if resolved.invoice_details.is_some() && !invoices.insert(resolved.destination.clone()) {
            return Err(SdkError::InvalidInput(format!(
                "Invoice appears more than once: {}",
                resolved.destination
            )));
        }
        recipients.push(resolved);
    }

    let totals = totals(&recipients)?;
    validate_output_cap(recipients.len(), totals.len())?;

    Ok(PrepareSendTokenBatchResponse { recipients, totals })
}

/// Rejects a batch that cannot fit the transaction output cap even before input
/// selection decides how many change outputs there really are.
///
/// The builder appends one change output per token with a remainder, and rejects
/// the transaction past the cap with a message from deep inside construction.
/// Failing here instead lets the caller act on it. The count is worst-case (one
/// change output assumed per token): whether a token produces change is only
/// known once inputs are selected at send time, and a verdict that depended on
/// the balance at prepare time could flip by the time the caller sends.
fn validate_output_cap(recipient_count: usize, token_count: usize) -> Result<(), SdkError> {
    let outputs = recipient_count
        .checked_add(token_count)
        .ok_or_else(|| SdkError::InvalidInput("Too many recipients".to_string()))?;
    if outputs > MAX_TOKEN_TX_OUTPUTS {
        return Err(SdkError::InvalidInput(format!(
            "A batch is limited to {MAX_TOKEN_TX_OUTPUTS} outputs, counting one change output \
             per token: {recipient_count} recipients across {token_count} token(s) needs {outputs}"
        )));
    }
    Ok(())
}

/// Resolves one requested recipient into the concrete token and amount that will
/// be sent, decoding an invoice destination when there is one.
async fn resolve(
    sdk: &BreezSdk,
    recipient: TokenBatchRecipient,
    identity_public_key: &str,
) -> Result<ResolvedTokenBatchRecipient, SdkError> {
    let (token_identifier, amount, invoice_details) =
        match sdk.parse(&recipient.destination).await? {
            InputType::SparkInvoice(details) => {
                let (token_identifier, amount) =
                    resolve_invoice(&details, &recipient, identity_public_key)?;
                (token_identifier, amount, Some(details))
            }
            InputType::SparkAddress(_) => {
                let token_identifier = recipient.token_identifier.clone().ok_or_else(|| {
                    SdkError::InvalidInput(format!(
                        "Token identifier is required for address {}",
                        recipient.destination
                    ))
                })?;
                let amount = recipient.amount.ok_or_else(|| {
                    SdkError::InvalidInput(format!(
                        "Amount is required for address {}",
                        recipient.destination
                    ))
                })?;
                (token_identifier, amount, None)
            }
            _ => {
                return Err(SdkError::InvalidInput(format!(
                    "A batch recipient must be a Spark address or a Spark invoice: {}",
                    recipient.destination
                )));
            }
        };

    if amount == 0 {
        return Err(SdkError::InvalidInput(format!(
            "Amount must be greater than 0 for {}",
            recipient.destination
        )));
    }

    Ok(ResolvedTokenBatchRecipient {
        destination: recipient.destination,
        amount,
        token_identifier,
        invoice_details,
    })
}

/// Reads the token and amount an invoice recipient pays, taking what the invoice
/// itself specifies and rejecting a request that contradicts it.
fn resolve_invoice(
    details: &SparkInvoiceDetails,
    recipient: &TokenBatchRecipient,
    identity_public_key: &str,
) -> Result<(String, u128), SdkError> {
    validation::validate_spark_invoice_payable(details, identity_public_key)?;

    let token_identifier = details.token_identifier.clone().ok_or_else(|| {
        SdkError::InvalidInput(format!(
            "A batch pays tokens, but this invoice requests sats: {}",
            recipient.destination
        ))
    })?;
    if let Some(requested) = &recipient.token_identifier
        && requested != &token_identifier
    {
        return Err(SdkError::InvalidInput(format!(
            "Requested token identifier does not match invoice token identifier: {}",
            recipient.destination
        )));
    }

    match (details.amount, recipient.amount) {
        (Some(invoice_amount), Some(requested)) if invoice_amount != requested => {
            Err(SdkError::InvalidInput(format!(
                "Requested amount does not match invoice amount: {}",
                recipient.destination
            )))
        }
        (Some(amount), _) | (None, Some(amount)) => Ok((token_identifier, amount)),
        (None, None) => Err(SdkError::InvalidInput(format!(
            "Amount is required when the invoice has no amount: {}",
            recipient.destination
        ))),
    }
}

/// Sums what the batch debits per token, in the order each token is first
/// requested.
fn totals(recipients: &[ResolvedTokenBatchRecipient]) -> Result<Vec<TokenBatchTotal>, SdkError> {
    let mut totals: Vec<TokenBatchTotal> = Vec::new();
    for recipient in recipients {
        if let Some(total) = totals
            .iter_mut()
            .find(|t| t.token_identifier == recipient.token_identifier)
        {
            total.amount = total.amount.checked_add(recipient.amount).ok_or_else(|| {
                SdkError::InvalidInput(format!(
                    "Total amount overflows for token {}",
                    recipient.token_identifier
                ))
            })?;
        } else {
            totals.push(TokenBatchTotal {
                token_identifier: recipient.token_identifier.clone(),
                amount: recipient.amount,
            });
        }
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BitcoinNetwork;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const TOKEN: &str = "btkn1token";
    const OTHER_TOKEN: &str = "btkn1other";

    fn invoice(amount: Option<u128>, token: Option<&str>) -> SparkInvoiceDetails {
        SparkInvoiceDetails {
            invoice: "sparkrt1invoice".to_string(),
            identity_public_key: "payee".to_string(),
            network: BitcoinNetwork::Regtest,
            amount,
            token_identifier: token.map(ToString::to_string),
            expiry_time: None,
            description: None,
            sender_public_key: None,
        }
    }

    fn recipient(amount: Option<u128>, token: Option<&str>) -> TokenBatchRecipient {
        TokenBatchRecipient {
            destination: "sparkrt1invoice".to_string(),
            amount,
            token_identifier: token.map(ToString::to_string),
        }
    }

    fn resolved(token: &str, amount: u128) -> ResolvedTokenBatchRecipient {
        ResolvedTokenBatchRecipient {
            destination: "sparkrt1address".to_string(),
            amount,
            token_identifier: token.to_string(),
            invoice_details: None,
        }
    }

    #[test_all]
    fn invoice_amount_wins_over_an_unset_request() {
        let resolved = resolve_invoice(
            &invoice(Some(250), Some(TOKEN)),
            &recipient(None, None),
            "us",
        )
        .unwrap();
        assert_eq!(resolved, (TOKEN.to_string(), 250));
    }

    #[test_all]
    fn amountless_invoice_takes_the_requested_amount() {
        let resolved = resolve_invoice(
            &invoice(None, Some(TOKEN)),
            &recipient(Some(70), None),
            "us",
        )
        .unwrap();
        assert_eq!(resolved, (TOKEN.to_string(), 70));
    }

    #[test_all]
    fn amountless_invoice_with_no_requested_amount_is_rejected() {
        let result = resolve_invoice(&invoice(None, Some(TOKEN)), &recipient(None, None), "us");
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn a_request_contradicting_the_invoice_amount_is_rejected() {
        let result = resolve_invoice(
            &invoice(Some(250), Some(TOKEN)),
            &recipient(Some(70), None),
            "us",
        );
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn a_request_contradicting_the_invoice_token_is_rejected() {
        let result = resolve_invoice(
            &invoice(Some(250), Some(TOKEN)),
            &recipient(None, Some(OTHER_TOKEN)),
            "us",
        );
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn a_sats_invoice_is_rejected() {
        let result = resolve_invoice(
            &invoice(Some(250), None),
            &recipient(None, Some(TOKEN)),
            "us",
        );
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn an_invoice_bound_to_another_sender_is_rejected() {
        let mut details = invoice(Some(250), Some(TOKEN));
        details.sender_public_key = Some("someone else".to_string());
        let result = resolve_invoice(&details, &recipient(None, None), "us");
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn totals_are_summed_per_token_in_first_requested_order() {
        let totals = totals(&[
            resolved(TOKEN, 100),
            resolved(OTHER_TOKEN, 5),
            resolved(TOKEN, 250),
        ])
        .unwrap();

        assert_eq!(totals.len(), 2);
        assert_eq!(totals[0].token_identifier, TOKEN);
        assert_eq!(totals[0].amount, 350);
        assert_eq!(totals[1].token_identifier, OTHER_TOKEN);
        assert_eq!(totals[1].amount, 5);
    }

    #[test_all]
    fn totals_reject_an_overflowing_token() {
        let result = totals(&[resolved(TOKEN, u128::MAX), resolved(TOKEN, 1)]);
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn output_cap_admits_a_batch_that_fits_with_change() {
        assert!(validate_output_cap(MAX_TOKEN_TX_OUTPUTS - 1, 1).is_ok());
    }

    #[test_all]
    fn output_cap_rejects_a_batch_one_change_output_over() {
        let result = validate_output_cap(MAX_TOKEN_TX_OUTPUTS, 1);
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn output_cap_counts_every_token_of_the_batch() {
        assert!(validate_output_cap(MAX_TOKEN_TX_OUTPUTS - 2, 2).is_ok());
        let result = validate_output_cap(MAX_TOKEN_TX_OUTPUTS - 1, 2);
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }

    #[test_all]
    fn output_cap_rejects_an_overflowing_count() {
        let result = validate_output_cap(usize::MAX, 1);
        assert!(matches!(result, Err(SdkError::InvalidInput(_))));
    }
}
