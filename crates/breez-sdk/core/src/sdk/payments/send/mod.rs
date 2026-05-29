pub(super) mod bitcoin_address;
pub(super) mod bolt11;
pub(super) mod lnurl_pay;
pub(super) mod spark_address;
pub(super) mod spark_invoice;

use crate::{
    SendPaymentMethod,
    error::SdkError,
    models::{SendPaymentRequest, SendPaymentResponse},
    sdk::BreezSdk,
};

pub(super) async fn send_internal(
    sdk: &BreezSdk,
    request: &SendPaymentRequest,
    amount_override: Option<u64>,
) -> Result<SendPaymentResponse, SdkError> {
    let amount = request.prepare_response.amount;
    let token_identifier = request.prepare_response.token_identifier.clone();

    match &request.prepare_response.payment_method {
        SendPaymentMethod::SparkAddress { address, .. } => {
            Box::pin(spark_address::send(
                sdk,
                address,
                token_identifier,
                amount_override.map_or(amount, u128::from),
                request.options.as_ref(),
                request.idempotency_key.clone(),
            ))
            .await
        }
        SendPaymentMethod::SparkInvoice {
            spark_invoice_details,
            ..
        } => {
            spark_invoice::send(
                sdk,
                &spark_invoice_details.invoice,
                request,
                amount_override.map_or(amount, u128::from),
            )
            .await
        }
        SendPaymentMethod::Bolt11Invoice {
            invoice_details,
            spark_transfer_fee_sats,
            lightning_fee_sats,
            ..
        } => {
            Box::pin(bolt11::send(
                sdk,
                invoice_details,
                *spark_transfer_fee_sats,
                *lightning_fee_sats,
                request,
                amount_override,
                amount,
            ))
            .await
        }
        SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
            bitcoin_address::send(sdk, address, fee_quote, request, amount_override).await
        }
    }
}
