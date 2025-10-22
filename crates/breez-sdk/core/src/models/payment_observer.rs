use std::sync::Arc;

use spark_wallet::{SparkAddress, TransferId, TransferObserverError};
use thiserror::Error;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ProvisionalPayment {
    /// Unique identifier for the payment
    pub payment_id: String,
    /// Amount in satoshis or token base units
    pub amount: u128,
    /// Details of the payment
    pub details: ProvisionalPaymentDetails,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ProvisionalPaymentDetails {
    Bitcoin {
        /// Onchain Bitcoin address
        withdrawal_address: String,
    },
    Lightning {
        /// BOLT11 invoice
        invoice: String,
    },
    Spark {
        /// Spark receiver public key
        receiver_public_key: String,
    },
    Token {
        /// Token identifier
        token_id: String,
        /// Spark receiver public key
        receiver_public_key: String,
    },
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
    /// Called before Lightning, Spark or onchain Bitcoin payments are made
    async fn before_send(
        &self,
        payments: Vec<ProvisionalPayment>,
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
            .before_send(vec![ProvisionalPayment {
                payment_id: transfer_id.to_string(),
                amount: u128::from(amount_sats),
                details: ProvisionalPaymentDetails::Bitcoin {
                    withdrawal_address: withdrawal_address.to_string(),
                },
            }])
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
            .before_send(vec![ProvisionalPayment {
                payment_id: transfer_id.to_string(),
                amount: u128::from(amount_sats),
                details: ProvisionalPaymentDetails::Lightning {
                    invoice: invoice.to_string(),
                },
            }])
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
            .before_send(
                receiver_outputs
                    .into_iter()
                    .enumerate()
                    .map(|(index, output)| ProvisionalPayment {
                        payment_id: format!("{tx_id}:{index}"),
                        amount: output.amount,
                        details: ProvisionalPaymentDetails::Token {
                            token_id: token_id.to_string(),
                            receiver_public_key: output.receiver_address.to_string(),
                        },
                    })
                    .collect(),
            )
            .await?)
    }

    async fn before_send_transfer(
        &self,
        transfer_id: &TransferId,
        receiver_address: &SparkAddress,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError> {
        Ok(self
            .inner
            .before_send(vec![ProvisionalPayment {
                payment_id: transfer_id.to_string(),
                amount: u128::from(amount_sats),
                details: ProvisionalPaymentDetails::Spark {
                    receiver_public_key: receiver_address.identity_public_key.to_string(),
                },
            }])
            .await?)
    }
}
