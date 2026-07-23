use spark_wallet::{SparkAddress, TokenRecipient};

use crate::{
    error::SdkError,
    events::SdkEvent,
    models::{
        ListPaymentsRequest, Payment, PaymentDetails, PaymentDetailsFilter,
        ResolvedTokenBatchRecipient, SendTokenBatchRequest, SendTokenBatchResponse,
    },
    sdk::BreezSdk,
    signer::ExternalPreparedTokenTransaction,
    utils::token::map_and_persist_token_transaction_payments,
};

pub(in crate::sdk) async fn send(
    sdk: &BreezSdk,
    request: SendTokenBatchRequest,
) -> Result<SendTokenBatchResponse, SdkError> {
    let recipients = to_token_recipients(&request.prepare_response.recipients)?;

    let token_transaction = sdk
        .spark_wallet
        .transfer_tokens(recipients, None, None)
        .await?;

    let payments = map_and_persist_token_transaction_payments(
        &sdk.spark_wallet,
        &sdk.storage,
        &token_transaction,
    )
    .await?;

    emit_payments(sdk, &payments).await;

    Ok(SendTokenBatchResponse { payments })
}

/// Turns the prepare response back into what the wallet pays. Invoices are
/// passed on as invoices rather than as their resolved address so the wallet
/// re-validates them and attaches them to the transaction: a prepare response is
/// caller-supplied, and an invoice may have expired since it was built.
pub(in crate::sdk) fn to_token_recipients(
    recipients: &[ResolvedTokenBatchRecipient],
) -> Result<Vec<TokenRecipient>, SdkError> {
    recipients
        .iter()
        .map(|recipient| {
            if recipient.invoice_details.is_some() {
                return Ok(TokenRecipient::Invoice {
                    invoice: recipient.destination.clone(),
                    amount: Some(recipient.amount),
                });
            }
            Ok(TokenRecipient::Address {
                token_id: recipient.token_identifier.clone(),
                amount: recipient.amount,
                receiver_address: recipient
                    .destination
                    .parse::<SparkAddress>()
                    .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?,
            })
        })
        .collect()
}

/// Broadcasts a batch package signed outside the SDK, returning every payment it
/// produced.
pub(in crate::sdk) async fn send_signed(
    sdk: &BreezSdk,
    token_context: &[u8],
    signed: &ExternalPreparedTokenTransaction,
) -> Result<Vec<Payment>, SdkError> {
    let token_transaction =
        super::spark_address::broadcast_signed_token_package(sdk, token_context, signed).await?;
    map_and_persist_token_transaction_payments(&sdk.spark_wallet, &sdk.storage, &token_transaction)
        .await
}

/// Recovers the payments of an already-published batch from the one payment id
/// recorded for its package.
///
/// The record holds a single id because a token package is keyed by its digest,
/// not by the transaction hash, which only exists after broadcast. The rest of
/// the batch is found through the transaction hash that first payment carries.
pub(in crate::sdk) async fn payments_for_published_batch(
    sdk: &BreezSdk,
    payment_id: String,
) -> Result<Vec<Payment>, SdkError> {
    let payment = sdk.storage.get_payment_by_id(payment_id).await?;
    let Some(PaymentDetails::Token { tx_hash, .. }) = payment.details.clone() else {
        return Ok(vec![payment]);
    };

    let mut payments = sdk
        .storage
        .list_payments(
            ListPaymentsRequest {
                payment_details_filter: Some(vec![PaymentDetailsFilter::Token {
                    conversion_refund_needed: None,
                    tx_hash: Some(tx_hash),
                    tx_type: None,
                }]),
                ..Default::default()
            }
            .into(),
        )
        .await?;
    // Listing is newest first and every payment in a batch shares a timestamp,
    // so restore the recipient order, which is the order of the outputs.
    payments.sort_by_key(|p| vout_of(&p.id));
    Ok(payments)
}

/// The vout a token payment id ends with, or `u32::MAX` for an id that carries
/// none, which sorts it last rather than reordering the batch around it.
fn vout_of(payment_id: &str) -> u32 {
    payment_id
        .rsplit_once(':')
        .and_then(|(_, vout)| vout.parse().ok())
        .unwrap_or(u32::MAX)
}

pub(in crate::sdk) async fn emit_payments(sdk: &BreezSdk, payments: &[Payment]) {
    for payment in payments {
        sdk.event_emitter
            .emit(&SdkEvent::from_payment(payment.clone()))
            .await;
    }
}
