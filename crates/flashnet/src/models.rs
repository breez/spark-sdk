//! Cross-cutting types shared between the AMM and Orchestra modules.

use spark_wallet::{TokenTransaction, WalletTransfer};

/// The asset transfer produced when sending the source-leg asset on a
/// Spark-side operation (an AMM swap-in or an Orchestra deposit). Carries the
/// rich wallet-side object so callers can record a `Payment` for the sent leg
/// without re-fetching it from the operator.
///
/// `Spark` is the larger variant; we don't box it because instances are
/// short-lived (constructed once, consumed by the caller almost
/// immediately) and adding indirection would only complicate consumers.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum AssetTransfer {
    Spark(WalletTransfer),
    Token(TokenTransaction),
}

impl AssetTransfer {
    /// Returns the operator-side identifier for this transfer — a Spark
    /// `transfer_id` or a token transaction hash, matching what the swap
    /// signing flow uses.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            AssetTransfer::Spark(t) => t.id.to_string(),
            AssetTransfer::Token(t) => t.hash.clone(),
        }
    }
}
