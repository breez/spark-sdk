use thiserror::Error;

#[derive(Error, Debug)]
pub enum SparkWalletError {
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Signer error: {0}")]
    SignerServiceError(#[from] spark::signer::SignerError),

    #[error("Deposit address used")]
    DepositAddressUsed,

    #[error("Deposit service error: {0}")]
    DepositServiceError(#[from] spark::services::DepositServiceError),

    #[error("Operator RPC error: {0}")]
    OperatorRpcError(#[from] spark::operator_rpc::OperatorRpcError),

    #[error("Generic error: {0}")]
    Generic(String),
}
