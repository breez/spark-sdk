use thiserror::Error;

#[derive(Error, Debug)]
pub enum SparkWalletError {
    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Insufficient funds")]
    InsufficientFunds,

    #[error("Invalid network")]
    InvalidNetwork,

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid output index")]
    InvalidOutputIndex,

    #[error("Leaves not found")]
    LeavesNotFound,

    #[error("Not a deposit output")]
    NotADepositOutput,

    #[error("Signer error: {0}")]
    SignerServiceError(#[from] spark::signer::SignerError),

    #[error("Deposit address used")]
    DepositAddressUsed,

    #[error("Operator RPC error: {0}")]
    OperatorRpcError(#[from] spark::operator::rpc::OperatorRpcError),

    #[error("Operator pool error: {0}")]
    OperatorPoolError(String),

    #[error("Address error: {0}")]
    AddressError(#[from] spark::address::error::AddressError),

    #[error("Tree service error: {0}")]
    TreeServiceError(#[from] spark::tree::TreeServiceError),

    #[error("Service error: {0}")]
    ServiceError(#[from] spark::services::ServiceError),

    #[error("Generic error: {0}")]
    Generic(String),
}
