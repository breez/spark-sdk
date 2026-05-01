use crate::error::SignerError;

/// Signer for external UTXO inputs in CPFP fee-bumping transactions.
///
/// The SDK calls this trait to sign the CPFP child transaction's external inputs.
/// The ephemeral anchor input is finalized by the SDK before this trait is called.
///
/// Implementations receive the PSBT serialized as bytes and must return the signed PSBT
/// (also serialized as bytes). The signer should:
/// 1. Deserialize the PSBT
/// 2. Sign all non-finalized inputs (skip inputs that already have `final_script_witness`)
/// 3. Set `final_script_witness` on signed inputs
/// 4. Serialize and return the signed PSBT
///
/// A default implementation ([`SingleKeySigner`](super::SingleKeySigner)) is provided for
/// signing P2WPKH and P2TR inputs with a single private key.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait CpfpSigner: Send + Sync {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError>;
}
