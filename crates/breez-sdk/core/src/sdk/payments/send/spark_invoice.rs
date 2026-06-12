use std::str::FromStr;

use spark_wallet::TransferId;

use crate::{
    ConversionOptions, ConversionPurpose, SparkInvoiceDetails,
    error::SdkError,
    models::{SendPaymentRequest, SendPaymentResponse},
    sdk::BreezSdk,
    sdk::payments::conversion,
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::token::map_and_persist_token_transaction,
};

pub(super) async fn send(
    sdk: &BreezSdk,
    invoice: &str,
    request: &SendPaymentRequest,
    amount: u128,
) -> Result<SendPaymentResponse, SdkError> {
    let transfer_id = request
        .idempotency_key
        .as_ref()
        .map(|key| TransferId::from_str(key))
        .transpose()?;

    let payment = match sdk
        .spark_wallet
        .fulfill_spark_invoice(invoice, Some(amount), transfer_id)
        .await?
    {
        spark_wallet::FulfillSparkInvoiceResult::Transfer(wallet_transfer) => {
            (*wallet_transfer).try_into()?
        }
        spark_wallet::FulfillSparkInvoiceResult::TokenTransaction(token_transaction) => {
            map_and_persist_token_transaction(&sdk.spark_wallet, &sdk.storage, &token_transaction)
                .await?
        }
    };

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(SendPaymentResponse { payment })
}

/// Runs the token conversion for a Spark-invoice send, returning the conversion
/// response and its purpose. The purpose is `SelfTransfer` when the invoice is
/// payable to our own identity (the conversion stays in-wallet), otherwise an
/// `OngoingPayment` toward the invoice.
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    spark_invoice_details: &SparkInvoiceDetails,
    conversion_amount: ConversionAmount,
    token_identifier: Option<&String>,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let purpose = conversion::conversion_purpose_for_identity(
        &sdk.spark_wallet.get_identity_public_key().to_string(),
        &spark_invoice_details.identity_public_key,
        spark_invoice_details.invoice.clone(),
    );
    let response = sdk
        .token_converter
        .convert(
            sdk.event_emitter.clone(),
            conversion_options,
            &purpose,
            token_identifier,
            conversion_amount,
            None,
        )
        .await?;
    Ok((response, purpose))
}
