use base64::Engine;
use bitcoin::hashes::{Hash, sha256};
use breez_sdk_common::lnurl::{
    error::LnurlError,
    pay::{AesSuccessActionDataResult, SuccessAction, SuccessActionProcessed},
};
use spark_wallet::SparkWallet;
use std::{str::FromStr, sync::Arc};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use x509_cert::Certificate;
use x509_cert::der::{Decode, asn1::ObjectIdentifier};

use crate::{
    PaymentDetails, WaitForPaymentIdentifier,
    error::SdkError,
    events::{EventListener, SdkEvent},
    models::Payment,
    persist::{Storage, UpdateWatchedAddressPayload},
    utils::deposit_address_watch::now_secs,
    utils::payments::update_balances,
};

/// Looks up the payment matching `identifier` from storage, if present.
///
/// Used as a fast-path check by `wait_for_incoming_payment` — if the
/// payment is already there and complete, callers can short-circuit
/// before starting a poll loop.
/// Returns `Ok(None)` when the row doesn't exist; surfaces real storage
/// errors so callers can bubble them rather than mask them.
pub(crate) async fn maybe_get_payment_from_storage(
    storage: &dyn Storage,
    identifier: &WaitForPaymentIdentifier,
) -> Result<Option<Payment>, SdkError> {
    match identifier {
        WaitForPaymentIdentifier::PaymentId(payment_id) => {
            Ok(storage.get_payment_by_id(payment_id.clone()).await.ok())
        }
        WaitForPaymentIdentifier::LightningReceive { invoice, .. } => {
            Ok(storage.get_payment_by_invoice(invoice.clone()).await?)
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

/// Returns a static deposit address, and starts watching it on-chain for
/// deposits still in the mempool.
///
/// When `new_address` is `true`, rotates to a fresh address (archives the
/// old one). The SO request creates one when no address exists yet.
///
/// When `new_address` is `false`, returns the existing address via
/// generate (which creates one on first call).
pub(crate) async fn get_deposit_address(
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    new_address: bool,
) -> Result<String, SdkError> {
    let address = if new_address {
        spark_wallet
            .rotate_static_deposit_address()
            .await?
            .to_string()
    } else {
        spark_wallet
            .generate_static_deposit_address()
            .await?
            .to_string()
    };
    watch_deposit_address(storage, &address).await;
    Ok(address)
}

/// Starts (or restarts) the on-chain watch on a deposit address about to be
/// handed out. Failures are logged, never propagated: the watch only makes a
/// deposit claimable a confirmation sooner, so it must not fail issuing the
/// address a payer is waiting for.
async fn watch_deposit_address(storage: &Arc<dyn Storage>, address: &str) {
    let Some(issued_at) = now_secs() else {
        warn!("Not watching deposit address {address}: the system clock is unusable");
        return;
    };
    if let Err(e) = storage
        .update_watched_deposit_address(
            address.to_string(),
            UpdateWatchedAddressPayload::Watch { issued_at },
        )
        .await
    {
        error!("Failed to watch deposit address {address}: {e}");
    }
}
