use std::sync::Arc;

use spark_wallet::{PublicKey, TransferId, TransferObserverError};
use thiserror::Error;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceiverTokenOutput {
    pub receiver_address: String,
    pub amount: u128,
}

impl From<spark_wallet::ReceiverTokenOutput> for ReceiverTokenOutput {
    fn from(value: spark_wallet::ReceiverTokenOutput) -> Self {
        ReceiverTokenOutput {
            receiver_address: value.receiver_address.to_string(),
            amount: value.amount,
        }
    }
}

#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PaymentObserverError {
    #[error("Service connectivity: {0}")]
    ServiceConnectivity(String),
    #[error("Generic: {0}")]
    Generic(String),
}

impl From<PaymentObserverError> for TransferObserverError {
    fn from(error: PaymentObserverError) -> Self {
        match error {
            PaymentObserverError::ServiceConnectivity(msg) => {
                TransferObserverError::ServiceConnectivity(msg)
            }
            PaymentObserverError::Generic(msg) => TransferObserverError::Generic(msg),
        }
    }
}

/// This interface is used to observe outgoing payments before Lightning, Spark and onchain Bitcoin payments.
/// If the implementation returns an error, the payment is cancelled.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait PaymentObserver: Send + Sync {
    /// Called before a cooperative exit is made
    async fn before_send_bitcoin(
        &self,
        payment_id: String,
        withdrawal_address: String,
        amount_sats: u64,
    ) -> Result<(), PaymentObserverError>;
    /// Called before a lightning payment is made
    async fn before_send_lightning(
        &self,
        payment_id: String,
        invoice: String,
        amount_sats: u64,
    ) -> Result<(), PaymentObserverError>;
    /// Called before a spark transfer is made
    async fn before_send_spark(
        &self,
        payment_id: String,
        receiver_public_key: String,
        amount_sats: u64,
    ) -> Result<(), PaymentObserverError>;
    /// Called before a spark token transaction is made
    async fn before_send_token(
        &self,
        tx_id: String,
        token_id: String,
        receiver_outputs: Vec<ReceiverTokenOutput>,
    ) -> Result<(), PaymentObserverError>;
}

pub(crate) struct SparkTransferObserver {
    inner: Arc<dyn PaymentObserver>,
}

impl SparkTransferObserver {
    pub fn new(inner: Arc<dyn PaymentObserver>) -> Self {
        Self { inner }
    }
}

#[macros::async_trait]
impl spark_wallet::TransferObserver for SparkTransferObserver {
    async fn before_coop_exit(
        &self,
        transfer_id: &TransferId,
        withdrawal_address: &bitcoin::Address,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError> {
        Ok(self
            .inner
            .before_send_bitcoin(
                transfer_id.to_string(),
                withdrawal_address.to_string(),
                amount_sats,
            )
            .await?)
    }
    async fn before_send_lightning_payment(
        &self,
        transfer_id: &TransferId,
        invoice: &str,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError> {
        Ok(self
            .inner
            .before_send_lightning(transfer_id.to_string(), invoice.to_string(), amount_sats)
            .await?)
    }

    async fn before_send_token(
        &self,
        tx_id: &bitcoin::Txid,
        token_id: &str,
        receiver_outputs: Vec<spark_wallet::ReceiverTokenOutput>,
    ) -> Result<(), TransferObserverError> {
        Ok(self
            .inner
            .before_send_token(
                tx_id.to_string(),
                token_id.to_string(),
                receiver_outputs.into_iter().map(Into::into).collect(),
            )
            .await?)
    }

    async fn before_send_transfer(
        &self,
        transfer_id: &TransferId,
        receiver_public_key: &PublicKey,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError> {
        Ok(self
            .inner
            .before_send_spark(
                transfer_id.to_string(),
                receiver_public_key.to_string(),
                amount_sats,
            )
            .await?)
    }
}
