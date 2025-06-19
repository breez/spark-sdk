use thiserror::Error;
use tonic::Status;

use crate::operator::rpc::OperatorRpcError;

#[derive(Debug, Error)]
pub enum DepositServiceError {
    #[error("bitcoin error: {0}")]
    BitcoinError(#[from] crate::bitcoin::BitcoinError),
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
    #[error("invalid output index")]
    InvalidOutputIndex,
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid signature share")]
    InvalidSignatureShare,
    #[error("invalid transaction")]
    InvalidTransaction,
    #[error("invalid verifying key")]
    InvalidVerifyingKey,
    #[error("request error: {0}")]
    RequestError(#[from] Status),
    #[error("signer error: {0}")]
    SignerError(#[from] crate::signer::SignerError),
    #[error("service connection error: {0}")]
    ServiceConnectionError(#[from] OperatorRpcError),
    #[error("unknown status: {0}")]
    UnknownStatus(String),
}
