use std::str::FromStr;

use bitcoin::hashes::sha256;
use platform_utils::time::Duration;
use spark_wallet::{SparkAddress, TransferId, TransferTokenOutput};

use crate::{
    ConversionOptions, ConversionPurpose, SendPaymentOptions, SparkHtlcOptions,
    error::SdkError,
    models::{Payment, SendPaymentResponse},
    sdk::BreezSdk,
    sdk::payments::conversion,
    token_conversion::{ConversionAmount, TokenConversionResponse},
    utils::token::map_and_persist_token_transaction,
};

pub(super) async fn send(
    sdk: &BreezSdk,
    address: &str,
    token_identifier: Option<String>,
    amount: u128,
    options: Option<&SendPaymentOptions>,
    idempotency_key: Option<String>,
) -> Result<SendPaymentResponse, SdkError> {
    let spark_address = address
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;

    // If HTLC options are provided, send an HTLC transfer
    if let Some(SendPaymentOptions::SparkAddress { htlc_options }) = options
        && let Some(htlc_options) = htlc_options
    {
        if token_identifier.is_some() {
            return Err(SdkError::InvalidInput(
                "Can't provide both token identifier and HTLC options".to_string(),
            ));
        }

        return send_htlc(
            sdk,
            &spark_address,
            amount.try_into()?,
            htlc_options,
            idempotency_key,
        )
        .await;
    }

    let payment = if let Some(identifier) = token_identifier {
        send_token_address(sdk, identifier, amount, spark_address).await?
    } else {
        let transfer_id = idempotency_key
            .as_ref()
            .map(|key| TransferId::from_str(key))
            .transpose()?;
        let transfer = sdk
            .spark_wallet
            .transfer(amount.try_into()?, &spark_address, transfer_id)
            .await?;
        transfer.try_into()?
    };

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(SendPaymentResponse { payment })
}

async fn send_htlc(
    sdk: &BreezSdk,
    address: &SparkAddress,
    amount_sat: u64,
    htlc_options: &SparkHtlcOptions,
    idempotency_key: Option<String>,
) -> Result<SendPaymentResponse, SdkError> {
    let payment_hash = sha256::Hash::from_str(&htlc_options.payment_hash)
        .map_err(|_| SdkError::InvalidInput("Invalid payment hash".to_string()))?;

    if htlc_options.expiry_duration_secs == 0 {
        return Err(SdkError::InvalidInput(
            "Expiry duration must be greater than 0".to_string(),
        ));
    }
    let expiry_duration = Duration::from_secs(htlc_options.expiry_duration_secs);

    let transfer_id = idempotency_key
        .as_ref()
        .map(|key| TransferId::from_str(key))
        .transpose()?;
    let transfer = sdk
        .spark_wallet
        .create_htlc(
            amount_sat,
            address,
            &payment_hash,
            expiry_duration,
            transfer_id,
        )
        .await?;

    let payment: Payment = transfer.try_into()?;

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(SendPaymentResponse { payment })
}

async fn send_token_address(
    sdk: &BreezSdk,
    token_identifier: String,
    amount: u128,
    receiver_address: SparkAddress,
) -> Result<Payment, SdkError> {
    let token_transaction = sdk
        .spark_wallet
        .transfer_tokens(
            vec![TransferTokenOutput {
                token_id: token_identifier,
                amount,
                receiver_address: receiver_address.clone(),
                spark_invoice: None,
            }],
            None,
            None,
        )
        .await?;

    map_and_persist_token_transaction(&sdk.spark_wallet, &sdk.storage, &token_transaction).await
}

/// Runs the token conversion for a Spark-address send, returning the conversion
/// response and its purpose. The purpose is `SelfTransfer` when the address is
/// our own identity (the conversion stays in-wallet), otherwise an
/// `OngoingPayment` toward the address.
pub(in crate::sdk::payments) async fn convert_token(
    sdk: &BreezSdk,
    conversion_options: &ConversionOptions,
    address: &str,
    conversion_amount: ConversionAmount,
    token_identifier: Option<&String>,
) -> Result<(TokenConversionResponse, ConversionPurpose), SdkError> {
    let spark_address = address
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
    let purpose = conversion::conversion_purpose_for_identity(
        &sdk.spark_wallet.get_identity_public_key().to_string(),
        &spark_address.identity_public_key.to_string(),
        address.to_string(),
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
