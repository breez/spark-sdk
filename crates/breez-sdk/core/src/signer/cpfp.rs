use crate::error::SignerError;

/// Signer for external UTXO inputs in CPFP fee-bumping transactions.
///
/// Signs the non-finalized inputs of a PSBT (serialized as bytes) and returns the
/// signed PSBT (also serialized as bytes).
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait CpfpSigner: Send + Sync {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError>;
}
