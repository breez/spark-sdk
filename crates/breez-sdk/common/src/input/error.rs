use thiserror::Error;

use crate::{error::ServiceConnectivityError, lnurl::error::LnurlError};

#[derive(Debug, Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum Bip21Error {
    #[error("bip21 contains invalid address")]
    InvalidAddress,
    #[error("bip21 contains invalid amount")]
    InvalidAmount,
    #[error("bip21 contains invalid parameter value for '{0}'")]
    InvalidParameter(String),
    #[error("bip21 parameter missing equals character")]
    MissingEquals,
    #[error("bip21 contains parameter '{0}' multiple times")]
    MultipleParams(String),
    #[error("bip21 contains unknown required parameter '{0}'")]
    UnknownRequiredParameter(String),
    #[error("bip21 does not contain any payment methods")]
    NoPaymentMethods,
}

impl Bip21Error {
    pub fn invalid_parameter(name: &str) -> Self {
        Self::InvalidParameter(name.to_string())
    }
    pub fn invalid_parameter_func<E>(name: &str) -> impl FnOnce(E) -> Self {
        |_| Self::invalid_parameter(name)
    }
    pub fn multiple_params(name: &str) -> Self {
        Self::MultipleParams(name.to_string())
    }
}

#[derive(Debug, Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum ParseError {
    #[error("empty input")]
    EmptyInput,
    #[error("Bip-21 error: {0}")]
    Bip21Error(Bip21Error),
    #[error("invalid input")]
    InvalidInput,
    #[error("Lnurl error: {0}")]
    LnurlError(LnurlError),
    #[error("Service connectivity error: {0}")]
    ServiceConnectivity(ServiceConnectivityError),
}

impl From<Bip21Error> for ParseError {
    fn from(value: Bip21Error) -> Self {
        Self::Bip21Error(value)
    }
}

impl From<LnurlError> for ParseError {
    fn from(value: LnurlError) -> Self {
        Self::LnurlError(value)
    }
}

impl From<ServiceConnectivityError> for ParseError {
    fn from(value: ServiceConnectivityError) -> Self {
        Self::ServiceConnectivity(value)
    }
}
