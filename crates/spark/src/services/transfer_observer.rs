use bitcoin::Address;
use thiserror::Error;

use crate::services::TransferId;

pub struct ReceiverTokenOutput {
    pub pay_request: String,
    pub amount: u128,
}

#[derive(Debug, Error, Clone)]
pub enum TransferObserverError {
    #[error("Service connectivity: {0}")]
    ServiceConnectivity(String),
    #[error("Error: {0}")]
    Generic(String),
}

#[macros::async_trait]
pub trait TransferObserver: Send + Sync {
    async fn before_coop_exit(
        &self,
        transfer_id: &TransferId,
        withdrawal_address: &Address,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError>;
    async fn before_send_lightning_payment(
        &self,
        transfer_id: &TransferId,
        invoice: &str,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError>;
    async fn before_send_token(
        &self,
        partial_tx_id: &str,
        token_id: &str,
        receiver_outputs: Vec<ReceiverTokenOutput>,
    ) -> Result<(), TransferObserverError>;
    /// `receiver_output_count` is the number of receiver outputs reported by `before_send_token`
    /// (the change output is excluded). Those outputs occupy vouts `0..receiver_output_count` in
    /// both the partial and the final transaction, so the observer can pair each provisional id
    /// `{partial_tx_id}:{i}` with its final id `{final_tx_id}:{i}`.
    async fn after_send_token(
        &self,
        _partial_tx_id: &str,
        _final_tx_id: &str,
        _receiver_output_count: usize,
    ) -> Result<(), TransferObserverError> {
        Ok(())
    }
    async fn before_send_transfer(
        &self,
        transfer_id: &TransferId,
        pay_request: &str,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError>;
}
