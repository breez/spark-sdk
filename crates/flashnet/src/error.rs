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

impl From<platform_utils::HttpError> for FlashnetError {
    fn from(err: platform_utils::HttpError) -> Self {
        Self::Network {
            code: err.status(),
            reason: err.to_string(),
        }
    }
}
