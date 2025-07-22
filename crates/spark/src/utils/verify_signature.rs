use bitcoin::{
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::{self, All, Message, PublicKey, ecdsa::Signature},
};

pub fn verify_signature_ecdsa<T: AsRef<[u8]>>(
    secp: &Secp256k1<All>,
    message: T,
    signature: &Signature,
    public_key: &PublicKey,
) -> Result<(), secp256k1::Error> {
    let digest = sha256::Hash::hash(message.as_ref());

    secp.verify_ecdsa(
        &Message::from_digest(digest.to_byte_array()),
        signature,
        public_key,
    )?;

    Ok(())
}
