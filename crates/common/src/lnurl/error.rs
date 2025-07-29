use std::array::TryFromSliceError;

use thiserror::Error;

use crate::error::ServiceConnectivityError;

pub type LnurlResult<T, E = LnurlError> = Result<T, E>;

#[derive(Debug, Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
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
    #[error("lnurl has unknown scheme")]
    InvalidUri,
}

impl From<TryFromSliceError> for LnurlError {
    fn from(err: TryFromSliceError) -> Self {
        Self::General(err.to_string())
    }
}
