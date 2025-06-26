use thiserror::Error;

use crate::operator::rpc::OperatorRpcError;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("frost error: {0}")]
    FrostError(#[from] frost_secp256k1_tr::Error),

    #[error("bitcoin io error: {0}")]
    BitcoinIOError(#[from] bitcoin::io::Error),
    #[error("invoice decoding error: {0}")]
    InvoiceDecodingError(String),
    #[error("validation error: {0}")]
    ValidationError(String),
    #[error("signer error: {0}")]
    SignerError(#[from] crate::signer::SignerError),
    #[error("service connection error: {0}")]
    ServiceConnectionError(#[from] OperatorRpcError),
    #[error("unknown status: {0}")]
    UnknownStatus(String),
    #[error("generic error: {0}")]
    Generic(String),
}
