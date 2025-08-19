use thiserror::Error;
use tonic::Status;

use crate::{operator::rpc::OperatorRpcError, tree::TreeNode};

#[derive(Debug, Error, Clone)]
pub enum ServiceError {
    // Deposit related errors
    #[error("deposit address already used")]
    DepositAddressUsed,
    #[error("invalid deposit address")]
    InvalidDepositAddress,
    #[error("invalid deposit address network")]
    InvalidDepositAddressNetwork,
    #[error("invalid identifier")]
    InvalidIdentifier,
    #[error("missing deposit address")]
    MissingDepositAddress,
    #[error("missing deposit address proof")]
    MissingDepositAddressProof,
    #[error("missing signing keyshare")]
    MissingSigningKeyshare,
    #[error("missing tree signatures")]
    MissingTreeSignatures,
    #[error("missing leaf id")]
    MissingLeafId,
    #[error("invalid deposit address proof")]
    InvalidDepositAddressProof,
    #[error("invalid node id: '{0}'")]
    InvalidNodeId(String),
    #[error("invalid output index")]
    InvalidOutputIndex,
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid signature share")]
    InvalidSignatureShare,
    #[error("invalid transaction")]
    InvalidTransaction,
    #[error("invalid network: {0}")]
    InvalidNetwork(i32),
    #[error("invalid verifying key")]
    InvalidVerifyingKey,
    #[error("not a deposit output")]
    NotADepositOutput,

    // Lightning related errors
    #[error("invoice decoding error: {0}")]
    InvoiceDecodingError(String),
    #[error("SSP swap error: {0}")]
    SSPswapError(String),
    #[error("preimage share store failed: {0}")]
    PreimageShareStoreFailed(String),
    #[error("payment not found")]
    PaymentNotFound,

    // Transfer related errors
    #[error("Failed to extend time lock: {0}")]
    ExtendTimeLockError(String),
    #[error("Transfer verification failed: {0}")]
    TransferVerificationError(String),
    #[error("Max retries exceeded")]
    MaxRetriesExceeded,
    #[error("No leaves to claim")]
    NoLeavesToClaim,
    #[error("Claim transfer failed: {0}")]
    ClaimTransferError(String),
    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),
    #[error("Transfer already claimed")]
    TransferAlreadyClaimed,

    // Swap related errors
    #[error("invalid amount")]
    InvalidAmount,
    #[error("insufficient funds")]
    InsufficientFunds,

    // Cooperative exit related errors
    #[error("Fee exceeds withdrawal amount")]
    InvalidFees,

    // Timelock manager related errors
    #[error("Partial check timelock error")]
    PartialCheckTimelockError(Vec<TreeNode>),

    // Common errors
    #[error("bitcoin error: {0}")]
    BitcoinError(#[from] crate::bitcoin::BitcoinError),
    #[error("frost error: {0}")]
    FrostError(#[from] frost_secp256k1_tr::Error),
    #[error("bitcoin io error: {0}")]
    BitcoinIOError(String),
    #[error("request error: {0}")]
    RequestError(Box<Status>),
    #[error("service provider error: {0}")]
    ServiceProviderError(#[from] crate::ssp::ServiceProviderError),
    #[error("validation error: {0}")]
    ValidationError(String),
    #[error("signer error: {0}")]
    SignerError(#[from] crate::signer::SignerError),
    #[error("service connection error: {0}")]
    ServiceConnectionError(Box<OperatorRpcError>),
    #[error("tree service error: {0}")]
    TreeServiceError(Box<crate::tree::TreeServiceError>),
    #[error("unknown status: {0}")]
    UnknownStatus(String),
    #[error("generic error: {0}")]
    Generic(String),
}

impl From<bitcoin::io::Error> for ServiceError {
    fn from(error: bitcoin::io::Error) -> Self {
        ServiceError::BitcoinIOError(error.to_string())
    }
}

impl From<Status> for ServiceError {
    fn from(status: Status) -> Self {
        ServiceError::RequestError(Box::new(status))
    }
}

impl From<OperatorRpcError> for ServiceError {
    fn from(error: OperatorRpcError) -> Self {
        ServiceError::ServiceConnectionError(Box::new(error))
    }
}

impl From<crate::tree::TreeServiceError> for ServiceError {
    fn from(error: crate::tree::TreeServiceError) -> Self {
        ServiceError::TreeServiceError(Box::new(error))
    }
}
