use bitcoin::{
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::{self, All, Message, PublicKey, ecdsa::RecoverableSignature},
};
use thiserror::Error;

use crate::signer::{RecoverableSignatureEncodeExt, SPARK_MESSAGE_PREFIX};

/// Verifies the message was signed by the given public key and the zbase32 encoded signature is valid.
///
/// The message is prefixed with "spark-message".
pub fn verify_recoverable_signature_ecdsa<T: AsRef<[u8]>>(
    secp: &Secp256k1<All>,
    message: T,
    signature: &str,
    public_key: &PublicKey,
) -> Result<(), VerifySignatureError> {
    let digest = sha256::Hash::hash(&[SPARK_MESSAGE_PREFIX, message.as_ref()].concat());

    let sig = RecoverableSignature::decode_compact(
        &zbase32::decode_full_bytes_str(signature)
            .map_err(VerifySignatureError::Zbase32DecodeError)?,
    )?;

    let recovered_pubkey =
        secp.recover_ecdsa(&Message::from_digest(digest.to_byte_array()), &sig)?;
    if recovered_pubkey != *public_key {
        return Err(VerifySignatureError::InvalidSignature);
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum VerifySignatureError {
    #[error("Failed to decode signature zbase32: {0}")]
    Zbase32DecodeError(&'static str),
    #[error("Secp256k1 error: {0}")]
    Secp256k1Error(#[from] secp256k1::Error),
    #[error("Invalid signature")]
    InvalidSignature,
}
