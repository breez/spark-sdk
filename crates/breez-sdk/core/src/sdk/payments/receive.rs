use std::str::FromStr;

use bitcoin::hashes::sha256;
use bitcoin::secp256k1::PublicKey;
use platform_utils::time::{Duration, SystemTime};
use spark_wallet::{InvoiceDescription, LightningReceivePayment, Preimage};

use crate::{
    ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse,
    error::SdkError,
    models::{Payment, ReceivePaymentMethod, ReceivePaymentRequest, ReceivePaymentResponse},
};

use super::super::{BreezSdk, helpers::get_deposit_address};

pub(super) async fn receive_payment(
    sdk: &BreezSdk,
    request: ReceivePaymentRequest,
) -> Result<ReceivePaymentResponse, SdkError> {
    sdk.maybe_ensure_spark_private_mode_initialized().await?;
    match request.payment_method {
        ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
            fee: 0,
            payment_request: sdk
                .spark_wallet
                .get_spark_address()?
                .to_address_string()
                .map_err(|e| {
                    SdkError::Generic(format!("Failed to convert Spark address to string: {e}"))
                })?,
        }),
        ReceivePaymentMethod::SparkInvoice {
            amount,
            token_identifier,
            expiry_time,
            description,
            sender_public_key,
        } => {
            let sender_public_key = sender_public_key
                .map(|key| PublicKey::from_str(&key))
                .transpose()
                .map_err(|_| SdkError::InvalidInput("Invalid sender public key".to_string()))?;
            let invoice = sdk
                .spark_wallet
                .create_spark_invoice(
                    amount,
                    token_identifier.clone(),
                    expiry_time
                        .map(|time| {
                            SystemTime::UNIX_EPOCH
                                .checked_add(Duration::from_secs(time))
                                .ok_or(SdkError::Generic("Invalid expiry time".to_string()))
                        })
                        .transpose()?,
                    description,
                    sender_public_key,
                )
                .await?;
            Ok(ReceivePaymentResponse {
                fee: 0,
                payment_request: invoice,
            })
        }
        ReceivePaymentMethod::BitcoinAddress { new_address } => {
            let address =
                get_deposit_address(&sdk.spark_wallet, new_address.unwrap_or(false)).await?;
            Ok(ReceivePaymentResponse {
                payment_request: address,
                fee: 0,
            })
        }
        ReceivePaymentMethod::Bolt11Invoice {
            description,
            amount_sats,
            expiry_secs,
            payment_hash,
        } => receive_bolt11_invoice(sdk, description, amount_sats, expiry_secs, payment_hash).await,
    }
}

pub(super) async fn claim_htlc_payment(
    sdk: &BreezSdk,
    request: ClaimHtlcPaymentRequest,
) -> Result<ClaimHtlcPaymentResponse, SdkError> {
    let preimage = Preimage::from_hex(&request.preimage)
        .map_err(|_| SdkError::InvalidInput("Invalid preimage".to_string()))?;
    let payment_hash = preimage.compute_hash();

    // Check if there is a claimable HTLC with the given payment hash
    let claimable_htlc_transfers = sdk.spark_wallet.list_claimable_htlc_transfers(None).await?;
    if !claimable_htlc_transfers
        .iter()
        .filter_map(|t| t.htlc_preimage_request.as_ref())
        .any(|p| p.payment_hash == payment_hash)
    {
        return Err(SdkError::InvalidInput(
            "No claimable HTLC with the given payment hash".to_string(),
        ));
    }

    let transfer = sdk.spark_wallet.claim_htlc(&preimage).await?;
    let payment: Payment = transfer.try_into()?;

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(ClaimHtlcPaymentResponse { payment })
}

pub(super) async fn receive_bolt11_invoice(
    sdk: &BreezSdk,
    description: String,
    amount_sats: Option<u64>,
    expiry_secs: Option<u32>,
    payment_hash: Option<String>,
) -> Result<ReceivePaymentResponse, SdkError> {
    let receive =
        receive_bolt11_invoice_inner(sdk, description, amount_sats, expiry_secs, payment_hash)
            .await?;
    Ok(ReceivePaymentResponse {
        payment_request: receive.invoice,
        fee: 0,
    })
}

/// Internal variant of [`receive_bolt11_invoice`] that keeps the
/// full SSP receive object (id + invoice + status + …). Used by
/// `lnurl_withdraw` to get the SSP id for the synchronous wait via
/// `WaitForPaymentIdentifier::LightningReceive`.
pub(super) async fn receive_bolt11_invoice_inner(
    sdk: &BreezSdk,
    description: String,
    amount_sats: Option<u64>,
    expiry_secs: Option<u32>,
    payment_hash: Option<String>,
) -> Result<LightningReceivePayment, SdkError> {
    let receive = if let Some(payment_hash_hex) = payment_hash {
        let hash = sha256::Hash::from_str(&payment_hash_hex)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid payment hash: {e}")))?;
        sdk.spark_wallet
            .create_hodl_lightning_invoice(
                amount_sats.unwrap_or_default(),
                Some(InvoiceDescription::Memo(description.clone())),
                hash,
                None,
                expiry_secs,
            )
            .await?
    } else {
        sdk.spark_wallet
            .create_lightning_invoice(
                amount_sats.unwrap_or_default(),
                Some(InvoiceDescription::Memo(description.clone())),
                None,
                expiry_secs,
                sdk.config.prefer_spark_over_lightning,
            )
            .await?
    };
    Ok(receive)
}
