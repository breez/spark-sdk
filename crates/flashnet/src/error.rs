use spark::signer::SignerError;
use spark_wallet::SessionStoreError;
use thiserror::Error;

use crate::AssetTransfer;

#[derive(Error, Debug, Clone)]
pub enum FlashnetError {
    #[error("{reason}")]
    Network { reason: String, code: Option<u16> },

    /// A pool execution failed after the outbound asset transfer was already
    /// made. `outbound_asset_transfer` carries the rich wallet-side object
    /// so the SDK can persist a `Payment` row + `ConversionInfo` for the
    /// stranded transfer without re-fetching it from the operators.
    /// Boxed because [`AssetTransfer`] (a `WalletTransfer` or
    /// `TokenTransaction` payload) is large enough to bloat every
    /// `Result<_, FlashnetError>` if inlined.
    #[error("Execution error: {source}")]
    Execution {
        #[source]
        source: Box<FlashnetError>,
        outbound_asset_transfer: Option<Box<AssetTransfer>>,
    },

    #[error("Session: {0}")]
    Session(#[from] SessionStoreError),

    #[error("Signer: {0}")]
    Signer(#[from] SignerError),

    #[error("Wallet: {0}")]
    Wallet(#[from] spark_wallet::SparkWalletError),

    #[error("Generic: {0}")]
    Generic(String),
}

impl FlashnetError {
    pub fn execution(
        source: FlashnetError,
        outbound_asset_transfer: Option<AssetTransfer>,
    ) -> Self {
        FlashnetError::Execution {
            source: Box::new(source),
            outbound_asset_transfer: outbound_asset_transfer.map(Box::new),
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
