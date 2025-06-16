use bitcoin::{PublicKey, key::Secp256k1, secp256k1::All};

pub enum CryptographyError {
    KeyCombinationError(String),
}

pub fn subtract_public_keys(
    a: &PublicKey,
    b: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<PublicKey, CryptographyError> {
    let combined = a
        .inner
        .combine(&b.inner.negate(secp))
        .map_err(|e| CryptographyError::KeyCombinationError(e.to_string()))?;
    Ok(combined.into())
}
