use std::sync::Arc;

use spark_wallet::{TransferId, TransferObserverError};
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
        /// Spark pay request being paid (either a Spark address or a Spark invoice)
        pay_request: String,
    },
    Token {
        /// Token identifier
        token_id: String,
        /// Spark pay request being paid (either a Spark address or a Spark invoice)
        pay_request: String,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentIdUpdate {
    /// Provisional payment id reported by `before_send`, in the form `{partial_tx_id}:{index}`
    pub provisional_payment_id: String,
    /// Final payment id once the transaction is broadcast, in the form `{final_tx_id}:{vout}`
    pub final_payment_id: String,
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

/// This interface is used to observe outgoing Lightning, Spark, onchain Bitcoin and token payments.
///
/// `before_send` is called before a payment is made; if the implementation returns an error the
/// payment is cancelled. `after_send` is called after a token payment has been broadcast to report
/// its final payment id; it cannot cancel the payment and any error it returns is ignored.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait PaymentObserver: Send + Sync {
    /// Called before Lightning, Spark, onchain Bitcoin or token payments are made
    async fn before_send(
        &self,
        payments: Vec<ProvisionalPayment>,
    ) -> Result<(), PaymentObserverError>;
    /// Called after a token payment has been broadcast, mapping each provisional payment id
    /// reported by `before_send` to its final payment id
    async fn after_send(&self, updates: Vec<PaymentIdUpdate>) -> Result<(), PaymentObserverError>;
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
        partial_tx_id: &str,
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
                        payment_id: format!("{partial_tx_id}:{index}"),
                        amount: output.amount,
                        details: ProvisionalPaymentDetails::Token {
                            token_id: token_id.to_string(),
                            pay_request: output.pay_request,
                        },
                    })
                    .collect(),
            )
            .await?)
    }

    async fn before_send_transfer(
        &self,
        transfer_id: &TransferId,
        receiver_address: &str,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError> {
        Ok(self
            .inner
            .before_send(vec![ProvisionalPayment {
                payment_id: transfer_id.to_string(),
                amount: u128::from(amount_sats),
                details: ProvisionalPaymentDetails::Spark {
                    pay_request: receiver_address.to_string(),
                },
            }])
            .await?)
    }

    async fn after_send_token(
        &self,
        partial_tx_id: &str,
        final_tx_id: &str,
        receiver_output_count: usize,
    ) -> Result<(), TransferObserverError> {
        // Pair each provisional id minted by before_send_token with its final id. The receiver
        // outputs keep their order (and vout) across the partial and final transaction, so index i
        // maps to vout i.
        let updates = (0..receiver_output_count)
            .map(|i| PaymentIdUpdate {
                provisional_payment_id: format!("{partial_tx_id}:{i}"),
                final_payment_id: format!("{final_tx_id}:{i}"),
            })
            .collect();
        Ok(self.inner.after_send(updates).await?)
    }
}
