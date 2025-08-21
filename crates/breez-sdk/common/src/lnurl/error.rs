use std::array::TryFromSliceError;

use thiserror::Error;

use crate::{error::ServiceConnectivityError, invoice::InvoiceError};

pub type LnurlResult<T, E = LnurlError> = Result<T, E>;

#[derive(Debug, Error, Clone)]
pub enum LnurlError {
    #[error("lnurl missing k1 parameter")]
    MissingK1,
    #[error("lnurl contains invalid k1 parameter")]
    InvalidK1,
    #[error("lnurl contains unsupported action")]
    UnsupportedAction,
    #[error("lnurl missing domain")]
    MissingDomain,
    #[error("error calling lnurl endpoint: {0}")]
    ServiceConnectivity(#[from] ServiceConnectivityError),
    #[error("endpoint error: {0}")]
    EndpointError(String),
    #[error("lnurl has http scheme without onion domain")]
    HttpSchemeWithoutOnionDomain,
    #[error("lnurl has https scheme with onion domain")]
    HttpsSchemeWithOnionDomain,
    #[error("lnurl error: {0}")]
    General(String),
    #[error("lnurl has unknown scheme")]
    UnknownScheme,
    #[error("lnurl has invalid uri: {0}")]
    InvalidUri(String),
    #[error("lnurl has invalid invoice: {0}")]
    InvalidInvoice(String),
    #[error("lnurl has invalid response: {0}")]
    InvalidResponse(String),
}

impl LnurlError {
    /// Returns a generic error message for the LNURL error
    pub fn general(msg: impl Into<String>) -> Self {
        Self::General(msg.into())
    }

    pub fn invalid_uri(msg: impl Into<String>) -> Self {
        Self::InvalidUri(msg.into())
    }
}

impl From<TryFromSliceError> for LnurlError {
    fn from(err: TryFromSliceError) -> Self {
        Self::General(err.to_string())
    }
}

impl From<InvoiceError> for LnurlError {
    fn from(value: InvoiceError) -> Self {
        LnurlError::InvalidInvoice(format!("{value}"))
    }
}

impl From<base64::DecodeError> for LnurlError {
    fn from(err: base64::DecodeError) -> Self {
        Self::General(err.to_string())
    }
}
