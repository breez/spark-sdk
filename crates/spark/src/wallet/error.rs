use thiserror::Error;

#[derive(Error, Debug)]
pub enum SparkWalletError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Signer error: {0}")]
    SignerServiceError(#[from] crate::signer::error::SignerError),

    #[error("Deposit address used")]
    DepositAddressUsed,

    #[error("Deposit service error: {0}")]
    DepositServiceError(#[from] crate::services::DepositServiceError),

    #[error("Operator RPC error: {0}")]
    OperatorRpcError(#[from] crate::operator_rpc::error::OperatorRpcError),

    #[error("Generic error: {0}")]
    Generic(String),
}
