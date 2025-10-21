use bitcoin::{Address, Txid};
use thiserror::Error;

use crate::{address::SparkAddress, services::TransferId};

pub struct ReceiverTokenOutput {
    pub receiver_address: SparkAddress,
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
        tx_id: &Txid,
        token_id: &str,
        receiver_outputs: Vec<ReceiverTokenOutput>,
    ) -> Result<(), TransferObserverError>;
    async fn before_send_transfer(
        &self,
        transfer_id: &TransferId,
        receiver_address: &SparkAddress,
        amount_sats: u64,
    ) -> Result<(), TransferObserverError>;
}
