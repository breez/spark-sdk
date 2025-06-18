use bitcoin::{key::Secp256k1, secp256k1::All, secp256k1::PublicKey};

pub enum CryptographyError {
    KeyCombinationError(String),
}

pub fn subtract_public_keys(
    a: &PublicKey,
    b: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<PublicKey, CryptographyError> {
    let negated = b.negate(secp);
    a.combine(&negated)
        .map_err(|e| CryptographyError::KeyCombinationError(e.to_string()))
}
