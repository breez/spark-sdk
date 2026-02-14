//! HTTP error types for platform-utils.

use thiserror::Error;

/// HTTP client error type with rich variants for different error conditions.
#[derive(Clone, Debug, Error)]
pub enum HttpError {
    #[error("Builder error: {0}")]
    Builder(String),
    #[error("Redirect error: {0}")]
    Redirect(String),
    #[error("Status error: {status} - {body}")]
    Status { status: u16, body: String },
    #[error("Timeout error: {0}")]
    Timeout(String),
    #[error("Request error: {0}")]
    Request(String),
    #[error("Connect error: {0}")]
    Connect(String),
    #[error("Body error: {0}")]
    Body(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Json error: {0}")]
    Json(String),
    #[error("Other error: {0}")]
    Other(String),
}

impl HttpError {
    /// Returns the HTTP status code if this error contains one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}

// Native: bitreq error conversion
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl From<bitreq::Error> for HttpError {
    fn from(err: bitreq::Error) -> Self {
        let err_str = format!("{err:?}");
        match &err {
            bitreq::Error::IoError(io_err) => {
                // Check if it's a timeout error
                if io_err.kind() == std::io::ErrorKind::TimedOut {
                    Self::Timeout(err_str)
                } else {
                    Self::Connect(err_str)
                }
            }
            bitreq::Error::InvalidUtf8InBody(_) | bitreq::Error::InvalidUtf8InResponse => {
                Self::Decode(err_str)
            }
            // Redirect-related errors
            bitreq::Error::TooManyRedirections
            | bitreq::Error::InfiniteRedirectionLoop
            | bitreq::Error::RedirectLocationMissing => Self::Redirect(err_str),
            // Connection errors
            bitreq::Error::AddressNotFound => Self::Connect(err_str),
            // Request/URL errors
            bitreq::Error::InvalidUrl(_) => Self::Request(err_str),
            // Body errors
            bitreq::Error::BodyOverflow => Self::Body(err_str),
            // Other errors
            bitreq::Error::Other(msg) => Self::Other((*msg).to_string()),
            _ => Self::Other(err_str),
        }
    }
}

// WASM: reqwest error conversion
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        let mut err_str = err.to_string();
        let mut walk: &dyn std::error::Error = &err;
        while let Some(src) = walk.source() {
            err_str.push_str(format!(" : {src}").as_str());
            walk = src;
        }
        if err.is_builder() {
            Self::Builder(err_str)
        } else if err.is_redirect() {
            Self::Redirect(err_str)
        } else if err.is_status() {
            Self::Status {
                status: err.status().unwrap_or_default().into(),
                body: err_str,
            }
        } else if err.is_timeout() {
            Self::Timeout(err_str)
        } else if err.is_request() {
            Self::Request(err_str)
        } else if err.is_body() {
            Self::Body(err_str)
        } else if err.is_decode() {
            Self::Decode(err_str)
        } else {
            Self::Other(err_str)
        }
    }
}
