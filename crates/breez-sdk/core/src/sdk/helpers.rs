use base64::Engine;
use bitcoin::hashes::{Hash, sha256};
use breez_sdk_common::lnurl::{
    error::LnurlError,
    pay::{AesSuccessActionDataResult, SuccessAction, SuccessActionProcessed},
};
use spark_wallet::SparkWallet;
use std::{str::FromStr, sync::Arc};
use tokio::sync::mpsc;
use tracing::{error, info};
use x509_parser::parse_x509_certificate;

use crate::{
    PaymentDetails, WaitForPaymentIdentifier,
    error::SdkError,
    events::{EventListener, SdkEvent},
    models::Payment,
    persist::{CachedAccountInfo, ObjectCacheRepository, StaticDepositAddress, Storage},
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

pub(crate) fn validate_breez_api_key(api_key: &str) -> Result<(), SdkError> {
    let api_key_decoded = base64::engine::general_purpose::STANDARD
        .decode(api_key.as_bytes())
        .map_err(|err| {
            SdkError::Generic(format!(
                "Could not base64 decode the Breez API key: {err:?}"
            ))
        })?;
    let (_rem, cert) = parse_x509_certificate(&api_key_decoded).map_err(|err| {
        SdkError::Generic(format!("Invalid certificate for Breez API key: {err:?}"))
    })?;

    let issuer = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok());
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

/// Gets an existing deposit address from cache/network or creates a new one.
///
/// This helper is used by both `receive_payment(BitcoinAddress)` and `buy_bitcoin`.
pub(crate) async fn get_or_create_deposit_address(
    spark_wallet: &SparkWallet,
    storage: Arc<dyn Storage>,
    is_static: bool,
) -> Result<String, SdkError> {
    let object_repository = ObjectCacheRepository::new(storage);

    // First lookup in storage cache
    if let Some(static_deposit_address) = object_repository.fetch_static_deposit_address().await? {
        return Ok(static_deposit_address.address);
    }

    // Then query existing addresses
    let deposit_addresses = spark_wallet.list_static_deposit_addresses(None).await?;

    // Use existing address or generate a new one
    let address = match deposit_addresses.items.last() {
        Some(address) => address.to_string(),
        None => spark_wallet
            .generate_deposit_address(is_static)
            .await?
            .to_string(),
    };

    // Cache the address
    object_repository
        .save_static_deposit_address(&StaticDepositAddress {
            address: address.clone(),
        })
        .await?;

    Ok(address)
}
