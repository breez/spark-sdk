/// Inline ECIES implementation compatible with the `ecies` 0.2.x crate (pure / aes-rust feature,
/// default configuration: uncompressed ephemeral key, uncompressed HKDF points, 16-byte nonce).
///
/// Wire format:
///   `ephemeral_pk` (65 B, uncompressed) || nonce (16 B) || GCM-tag (16 B) || ciphertext
///
/// KDF: HKDF-SHA256, IKM = `sender_pk_uncompressed` (65 B) || `ecdh_point_uncompressed` (65 B),
///      salt = none, info = empty → 32-byte AES-256 key.
use aes_gcm::{
    AesGcm, Key, KeyInit,
    aead::{AeadInPlace, consts::U16, generic_array::GenericArray},
    aes::Aes256,
};
use hkdf::Hkdf;
use k256::{
    AffinePoint, ProjectivePoint, PublicKey, SecretKey, elliptic_curve::sec1::ToEncodedPoint,
};
use rand::{RngCore, rngs::OsRng};
use sha2::Sha256;
use thiserror::Error;

/// AES-256-GCM with 16-byte nonce (matches ecies default, distinct from the standard 12-byte one).
type Cipher = AesGcm<Aes256, U16>;
type Nonce = GenericArray<u8, U16>;

const UNCOMPRESSED_PK_SIZE: usize = 65;
const NONCE_SIZE: usize = 16;
const TAG_SIZE: usize = 16;
const OVERHEAD: usize = UNCOMPRESSED_PK_SIZE + NONCE_SIZE + TAG_SIZE;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("invalid message")]
    InvalidMessage,
    #[error("ECDH produced the identity point")]
    EcdhFailed,
    #[error("HKDF key derivation failed")]
    KdfFailed,
}

/// Encrypt `msg` for `receiver_pub_bytes` (compressed 33 B or uncompressed 65 B SEC1 pubkey).
pub fn encrypt(receiver_pub_bytes: &[u8], msg: &[u8]) -> Result<Vec<u8>, Error> {
    let receiver_pubkey =
        PublicKey::from_sec1_bytes(receiver_pub_bytes).map_err(|_| Error::InvalidPublicKey)?;

    let ephemeral_seckey = SecretKey::random(&mut OsRng);
    let ephemeral_pubkey = ephemeral_seckey.public_key();

    let sym_key = derive_sym_key(&ephemeral_pubkey, &ephemeral_seckey, &receiver_pubkey)?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);

    let cipher = Cipher::new(Key::<Cipher>::from_slice(&sym_key));
    let mut ciphertext = msg.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce_bytes), b"", &mut ciphertext)
        .map_err(|_| Error::InvalidMessage)?;

    let mut out = Vec::with_capacity(OVERHEAD.saturating_add(msg.len()));
    out.extend_from_slice(ephemeral_pubkey.to_encoded_point(false).as_bytes());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(tag.as_slice());
    out.extend_from_slice(&ciphertext);

    Ok(out)
}

/// Decrypt `msg` using the 32-byte raw secret key `receiver_sec_bytes`.
pub fn decrypt(receiver_sec_bytes: &[u8], msg: &[u8]) -> Result<Vec<u8>, Error> {
    if msg.len() < OVERHEAD {
        return Err(Error::InvalidMessage);
    }

    let receiver_sk =
        SecretKey::from_slice(receiver_sec_bytes).map_err(|_| Error::InvalidMessage)?;
    let ephemeral_pk = PublicKey::from_sec1_bytes(&msg[..UNCOMPRESSED_PK_SIZE])
        .map_err(|_| Error::InvalidPublicKey)?;

    let sym_key = derive_sym_key(&ephemeral_pk, &receiver_sk, &ephemeral_pk)?;

    let nonce = Nonce::from_slice(&msg[UNCOMPRESSED_PK_SIZE..UNCOMPRESSED_PK_SIZE + NONCE_SIZE]);
    let tag =
        GenericArray::<u8, U16>::from_slice(&msg[UNCOMPRESSED_PK_SIZE + NONCE_SIZE..OVERHEAD]);
    let mut plaintext = msg[OVERHEAD..].to_vec();

    let cipher = Cipher::new(Key::<Cipher>::from_slice(&sym_key));
    cipher
        .decrypt_in_place_detached(nonce, b"", &mut plaintext, tag)
        .map_err(|_| Error::InvalidMessage)?;

    Ok(plaintext)
}

/// ECDH + HKDF-SHA256 key derivation.
///
/// - `sender_pk`: the ephemeral public key (always)
/// - `scalar_sk`:  the private key performing the ECDH multiply
/// - `ecdh_pk`:    the public key being multiplied (receiver pk on encrypt, ephemeral pk on decrypt)
fn derive_sym_key(
    sender_pk: &PublicKey,
    scalar_sk: &SecretKey,
    ecdh_pk: &PublicKey,
) -> Result<[u8; 32], Error> {
    let scalar = scalar_sk.to_nonzero_scalar();
    // `ProjectivePoint * Scalar` is EC scalar multiplication — integer overflow doesn't apply.
    #[allow(clippy::arithmetic_side_effects)]
    let shared_projective = ProjectivePoint::from(*ecdh_pk.as_affine()) * *scalar;
    let shared_affine = AffinePoint::from(shared_projective);
    let shared_pk = PublicKey::from_affine(shared_affine).map_err(|_| Error::EcdhFailed)?;

    let sender_bytes = sender_pk.to_encoded_point(false); // 65 B: 0x04 || x || y
    let shared_bytes = shared_pk.to_encoded_point(false); // 65 B: 0x04 || x || y

    let mut ikm = [0u8; 130]; // 65 + 65
    ikm[..65].copy_from_slice(sender_bytes.as_bytes());
    ikm[65..].copy_from_slice(shared_bytes.as_bytes());

    let h = Hkdf::<Sha256>::new(None, &ikm);
    let mut out = [0u8; 32];
    h.expand(&[], &mut out).map_err(|_| Error::KdfFailed)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip: anything encrypted with a pubkey decrypts with the matching privkey.
    #[test]
    fn test_round_trip() {
        let sk = SecretKey::random(&mut OsRng);
        let pk = sk.public_key();

        let msg = b"hello world";
        let ct = encrypt(pk.to_encoded_point(false).as_bytes(), msg).unwrap();
        let pt = decrypt(&sk.to_bytes(), &ct).unwrap();

        assert_eq!(pt, msg);
    }

    /// Both compressed (33 B) and uncompressed (65 B) receiver pubkey encodings are accepted.
    #[test]
    fn test_compressed_and_uncompressed_pubkey() {
        let sk = SecretKey::random(&mut OsRng);
        let pk = sk.public_key();
        let sk_bytes = sk.to_bytes();

        let msg = b"key encoding test";

        let ct_uncompressed = encrypt(pk.to_encoded_point(false).as_bytes(), msg).unwrap();
        assert_eq!(decrypt(&sk_bytes, &ct_uncompressed).unwrap(), msg);

        let ct_compressed = encrypt(pk.to_encoded_point(true).as_bytes(), msg).unwrap();
        assert_eq!(decrypt(&sk_bytes, &ct_compressed).unwrap(), msg);
    }

    /// Known-vector from the ecies 0.2.x crate test suite (secp256k1, pure/aes-rust feature,
    /// default config: uncompressed ephemeral key, 16-byte nonce, no short-nonce, no xchacha20).
    /// Source: <https://github.com/ecies/rs/blob/947eeb682558667a35a8a7ffb70f5c58c390e8a0/src/elliptic/secp256k1.rs#L117>
    #[test]
    fn test_known_vector_compatibility() {
        let sk_hex = "e520872701d9ec44dbac2eab85512ad14ad0c42e01de56d7b528abd8524fcb47";
        let ct_hex = concat!(
            "047be1885aeb48d4d4db0c992996725d3264784fef88c5b60782f8d0f940c21",
            "3227fc3f904f846d5ec3d0fba6653754501e8ebadc421aa3892a20fef33cff0",
            "206047058a4cfb4efbeae96b2d019b4ab2edce33328748a0d008a69c8f5816b",
            "72d45bd9b5a41bb6ea0127ab23057ec6fcd"
        );

        let sk = hex::decode(sk_hex).unwrap();
        let ct = hex::decode(ct_hex).unwrap();

        let plaintext = decrypt(&sk, &ct).unwrap();
        assert_eq!(plaintext, "hello world🌍".as_bytes());
    }

    /// Decryption with the wrong private key must fail (GCM tag rejects it).
    #[test]
    fn test_wrong_key_fails() {
        let sk = SecretKey::random(&mut OsRng);
        let pk = sk.public_key();
        let other_sk = SecretKey::random(&mut OsRng);

        let ct = encrypt(pk.to_encoded_point(false).as_bytes(), b"secret").unwrap();
        let result = decrypt(&other_sk.to_bytes(), &ct);

        assert!(matches!(result, Err(Error::InvalidMessage)));
    }

    /// Ciphertexts shorter than OVERHEAD bytes must be rejected immediately.
    #[test]
    fn test_truncated_ciphertext_fails() {
        assert!(matches!(
            decrypt(&[1u8; 32], &[]),
            Err(Error::InvalidMessage)
        ));
        assert!(matches!(
            decrypt(&[1u8; 32], &[0u8; OVERHEAD - 1]),
            Err(Error::InvalidMessage)
        ));
    }
}
