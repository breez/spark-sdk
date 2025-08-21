use thiserror::Error;

#[derive(Clone, Debug, Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum ServiceConnectivityError {
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

impl From<reqwest::Error> for ServiceConnectivityError {
    fn from(err: reqwest::Error) -> Self {
        let err_str = err.to_string();
        #[allow(unused_mut)]
        let mut res = if err.is_builder() {
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
        };
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        if err.is_connect() {
            res = Self::Connect(err.to_string())
        }
        res
    }
}
