use base64::Engine;
use bitcoin::hashes::{Hash, sha256};
use breez_sdk_common::lnurl::{
    error::LnurlError,
    pay::{AesSuccessActionDataResult, SuccessAction, SuccessActionProcessed},
};
use spark_wallet::SparkWallet;
use std::{str::FromStr, sync::Arc};
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use x509_cert::Certificate;
use x509_cert::der::{Decode, asn1::ObjectIdentifier};

use crate::{
    PaymentDetails, WaitForPaymentIdentifier,
    error::SdkError,
    events::{EventListener, SdkEvent},
    models::Payment,
    persist::{CachedAccountInfo, ObjectCacheRepository, Storage},
};

pub(crate) fn is_payment_match(payment: &Payment, identifier: &WaitForPaymentIdentifier) -> bool {
    match identifier {
        WaitForPaymentIdentifier::PaymentId(payment_id) => payment.id == *payment_id,
        WaitForPaymentIdentifier::PaymentRequest(payment_request) => {
            if let Some(details) = &payment.details {
                match details {
                    PaymentDetails::Lightning { invoice, .. } => {
                        invoice.to_lowercase() == payment_request.to_lowercase()
                    }
                    PaymentDetails::Spark {
                        invoice_details: invoice,
                        ..
                    }
                    | PaymentDetails::Token {
                        invoice_details: invoice,
                        ..
                    } => {
                        if let Some(invoice) = invoice {
                            invoice.invoice.to_lowercase() == payment_request.to_lowercase()
                        } else {
                            false
                        }
                    }
                    PaymentDetails::Withdraw { tx_id: _ }
                    | PaymentDetails::Deposit { tx_id: _ } => false,
                }
            } else {
                false
            }
        }
    }
}

pub(crate) struct BalanceWatcher {
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
}

impl BalanceWatcher {
    pub(crate) fn new(spark_wallet: Arc<SparkWallet>, storage: Arc<dyn Storage>) -> Self {
        Self {
            spark_wallet,
            storage,
        }
    }
}

#[macros::async_trait]
impl EventListener for BalanceWatcher {
    async fn on_event(&self, event: SdkEvent) {
        match event {
            SdkEvent::PaymentSucceeded { .. } | SdkEvent::ClaimedDeposits { .. } => {
                match update_balances(self.spark_wallet.clone(), self.storage.clone()).await {
                    Ok(()) => info!("Balance updated successfully"),
                    Err(e) => error!("Failed to update balance: {e:?}"),
                }
            }
            _ => {}
        }
    }
}

pub(crate) async fn update_balances(
    spark_wallet: Arc<SparkWallet>,
    storage: Arc<dyn Storage>,
) -> Result<(), SdkError> {
    let balance_sats = spark_wallet.get_balance().await?;
    let token_balances = spark_wallet
        .get_token_balances()
        .await?
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect();
    let object_repository = ObjectCacheRepository::new(storage.clone());

    object_repository
        .save_account_info(&CachedAccountInfo {
            balance_sats,
            token_balances,
        })
        .await?;
    let identity_public_key = spark_wallet.get_identity_public_key();
    info!(
        "Balance updated successfully {} for identity {}",
        balance_sats, identity_public_key
    );
    Ok(())
}

pub(crate) struct InternalEventListener {
    tx: mpsc::Sender<SdkEvent>,
}

impl InternalEventListener {
    #[allow(unused)]
    pub fn new(tx: mpsc::Sender<SdkEvent>) -> Self {
        Self { tx }
    }
}

#[macros::async_trait]
impl EventListener for InternalEventListener {
    async fn on_event(&self, event: SdkEvent) {
        let _ = self.tx.send(event).await;
    }
}

pub(crate) fn process_success_action(
    payment: &Payment,
    success_action: Option<&SuccessAction>,
) -> Result<Option<SuccessActionProcessed>, LnurlError> {
    let Some(success_action) = success_action else {
        return Ok(None);
    };

    let data = match success_action {
        SuccessAction::Aes { data } => data,
        SuccessAction::Message { data } => {
            return Ok(Some(SuccessActionProcessed::Message { data: data.clone() }));
        }
        SuccessAction::Url { data } => {
            return Ok(Some(SuccessActionProcessed::Url { data: data.clone() }));
        }
    };

    let Some(PaymentDetails::Lightning { htlc_details, .. }) = &payment.details else {
        return Err(LnurlError::general(format!(
            "Invalid payment type: expected type `PaymentDetails::Lightning`, got payment details {:?}.",
            payment.details
        )));
    };

    let Some(preimage) = &htlc_details.preimage else {
        return Ok(None);
    };

    let preimage =
        sha256::Hash::from_str(preimage).map_err(|_| LnurlError::general("Invalid preimage"))?;
    let preimage = preimage.as_byte_array();
    let result: AesSuccessActionDataResult = match (data, preimage).try_into() {
        Ok(data) => AesSuccessActionDataResult::Decrypted { data },
        Err(e) => AesSuccessActionDataResult::ErrorStatus {
            reason: e.to_string(),
        },
    };

    Ok(Some(SuccessActionProcessed::Aes { result }))
}

// OID 2.5.4.3 = commonName
const OID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");

pub(crate) fn validate_breez_api_key(api_key: &str) -> Result<(), SdkError> {
    let api_key_decoded = base64::engine::general_purpose::STANDARD
        .decode(api_key.as_bytes())
        .map_err(|err| {
            SdkError::Generic(format!(
                "Could not base64 decode the Breez API key: {err:?}"
            ))
        })?;
    let cert = Certificate::from_der(&api_key_decoded).map_err(|err| {
        SdkError::Generic(format!("Invalid certificate for Breez API key: {err:?}"))
    })?;

    let issuer = cert
        .tbs_certificate
        .issuer
        .0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|atv| atv.oid == OID_COMMON_NAME)
        .and_then(|atv| str::from_utf8(atv.value.value()).ok());
    match issuer {
        Some(common_name) => {
            if !common_name.starts_with("Breez") {
                return Err(SdkError::Generic(
                    "Invalid certificate found for Breez API key: issuer mismatch. Please confirm that the certificate's origin is trusted"
                        .to_string()
                ));
            }
        }
        _ => {
            return Err(SdkError::Generic(
                "Could not parse Breez API key certificate: issuer is invalid or not found."
                    .to_string(),
            ));
        }
    }

    Ok(())
}

/// Returns a static deposit address.
///
/// When `new_address` is `true`, rotates to a fresh address (archives the
/// old one), falling back to generate when no address exists yet (gRPC
/// `NotFound`).
///
/// When `new_address` is `false`, returns the existing address via
/// generate (which creates one on first call).
pub(crate) async fn get_deposit_address(
    spark_wallet: &SparkWallet,
    new_address: bool,
) -> Result<String, SdkError> {
    if new_address {
        match spark_wallet.rotate_static_deposit_address().await {
            Ok(addr) => Ok(addr.to_string()),
            Err(e) if e.is_not_found() => {
                debug!("No existing deposit address found, generating a new one");
                Ok(spark_wallet
                    .generate_static_deposit_address()
                    .await?
                    .to_string())
            }
            Err(e) => Err(e.into()),
        }
    } else {
        Ok(spark_wallet
            .generate_static_deposit_address()
            .await?
            .to_string())
    }
}
