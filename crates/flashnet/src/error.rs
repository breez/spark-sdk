use spark::signer::SignerError;
use spark_wallet::SessionManagerError;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum FlashnetError {
    #[error("Network error: {reason} (code: {code:?})")]
    Network { reason: String, code: Option<u16> },

    #[error("Execution error: {source}")]
    Execution {
        #[source]
        source: Box<FlashnetError>,
        transaction_identifier: Option<String>,
    },

    #[error("Session: {0}")]
    Session(#[from] SessionManagerError),

    #[error("Signer: {0}")]
    Signer(#[from] SignerError),

    #[error("Wallet: {0}")]
    Wallet(#[from] spark_wallet::SparkWalletError),

    #[error("Generic: {0}")]
    Generic(String),
}

impl FlashnetError {
    pub fn execution(source: FlashnetError, transaction_identifier: Option<String>) -> Self {
        FlashnetError::Execution {
            source: Box::new(source),
            transaction_identifier,
        }
    }
}

impl From<reqwest::Error> for FlashnetError {
    fn from(err: reqwest::Error) -> Self {
        let mut err_str = err.to_string();
        let mut walk: &dyn std::error::Error = &err;
        while let Some(src) = walk.source() {
            err_str.push_str(format!(" : {src}").as_str());
            walk = src;
        }
        Self::Network {
            reason: err_str,
            code: err.status().map(|s| s.as_u16()),
        }
    }
}
