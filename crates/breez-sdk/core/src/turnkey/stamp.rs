//! Turnkey request "stamper": signs each request body with the API key and
//! produces the `X-Stamp` header Turnkey requires.
//!
//! Turnkey supports both secp256k1 and P-256 API keys. secp256k1 is always
//! available (reusing the SDK's `bitcoin`/`secp256k1` dependency); P-256
//! (Turnkey's console default) is supported when built with the `turnkey-p256`
//! feature, which pulls in the `p256` crate. The curve is detected from the
//! configured public key, so callers just provide their keypair.

use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{self, Message, PublicKey, Secp256k1, SecretKey};
use serde::Serialize;

use super::error::TurnkeyError;

const SIGNATURE_SCHEME_SECP256K1: &str = "SIGNATURE_SCHEME_TK_API_SECP256K1";
#[cfg(feature = "turnkey-p256")]
const SIGNATURE_SCHEME_P256: &str = "SIGNATURE_SCHEME_TK_API_P256";
const X_STAMP_HEADER: &str = "X-Stamp";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiStamp {
    public_key: String,
    signature: String,
    scheme: String,
}

/// The API keypair by curve. secp256k1 is always available; P-256 is gated on
/// the `turnkey-p256` feature.
enum StamperKey {
    Secp256k1 {
        secret_key: SecretKey,
        public_key: PublicKey,
        secp: Secp256k1<secp256k1::All>,
    },
    #[cfg(feature = "turnkey-p256")]
    P256(p256::ecdsa::SigningKey),
}

#[cfg(feature = "turnkey-p256")]
fn p256_public_key_hex(signing_key: &p256::ecdsa::SigningKey) -> String {
    hex::encode(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes(),
    )
}

/// Signs Turnkey request bodies with the configured API key.
pub(crate) struct ApiKeyStamper {
    key: StamperKey,
}

impl ApiKeyStamper {
    /// Builds a stamper from the hex-encoded API private key, selecting the curve
    /// whose derived compressed public key matches `expected_public_key_hex`.
    /// secp256k1 is tried first; P-256 only when the `turnkey-p256` feature is on.
    pub(crate) fn from_hex(
        private_key_hex: &str,
        expected_public_key_hex: &str,
    ) -> Result<Self, TurnkeyError> {
        if expected_public_key_hex.is_empty() {
            return Err(TurnkeyError::InvalidApiKey(
                "API public key is required to select the signing curve".to_string(),
            ));
        }
        let priv_bytes = hex::decode(private_key_hex)
            .map_err(|e| TurnkeyError::InvalidApiKey(format!("private key hex: {e}")))?;

        // secp256k1 (always available).
        if let Ok(secret_key) = SecretKey::from_slice(&priv_bytes) {
            let secp = Secp256k1::new();
            let public_key = secret_key.public_key(&secp);
            let derived = hex::encode(public_key.serialize());
            if derived.eq_ignore_ascii_case(expected_public_key_hex) {
                return Ok(Self {
                    key: StamperKey::Secp256k1 {
                        secret_key,
                        public_key,
                        secp,
                    },
                });
            }
        }

        // P-256 (feature-gated).
        #[cfg(feature = "turnkey-p256")]
        if let Ok(signing_key) = p256::ecdsa::SigningKey::from_slice(&priv_bytes) {
            let derived = p256_public_key_hex(&signing_key);
            if derived.eq_ignore_ascii_case(expected_public_key_hex) {
                return Ok(Self {
                    key: StamperKey::P256(signing_key),
                });
            }
        }

        Err(TurnkeyError::InvalidApiKey(mismatch_message(
            expected_public_key_hex,
        )))
    }

    fn public_key_hex(&self) -> String {
        match &self.key {
            StamperKey::Secp256k1 { public_key, .. } => hex::encode(public_key.serialize()),
            #[cfg(feature = "turnkey-p256")]
            StamperKey::P256(signing_key) => p256_public_key_hex(signing_key),
        }
    }

    /// Stamps `body`, returning the `(header_name, header_value)` to attach to
    /// the request. The value is a base64url-no-pad JSON object
    /// `{publicKey, signature, scheme}`; the signature is DER-encoded ECDSA over
    /// SHA-256 of the body.
    pub(crate) fn stamp(&self, body: &[u8]) -> Result<(String, String), TurnkeyError> {
        let (signature, scheme) = match &self.key {
            StamperKey::Secp256k1 {
                secret_key, secp, ..
            } => {
                let digest = sha256::Hash::hash(body);
                let message = Message::from_digest(digest.to_byte_array());
                let sig = secp.sign_ecdsa(&message, secret_key);
                (hex::encode(sig.serialize_der()), SIGNATURE_SCHEME_SECP256K1)
            }
            #[cfg(feature = "turnkey-p256")]
            StamperKey::P256(signing_key) => {
                use p256::ecdsa::signature::Signer;
                // P-256's associated digest is SHA-256, so this signs SHA-256(body).
                let sig: p256::ecdsa::Signature = signing_key.sign(body);
                (hex::encode(sig.to_der().as_bytes()), SIGNATURE_SCHEME_P256)
            }
        };
        let stamp = ApiStamp {
            public_key: self.public_key_hex(),
            signature,
            scheme: scheme.to_string(),
        };
        let json =
            serde_json::to_vec(&stamp).map_err(|e| TurnkeyError::Serialize(e.to_string()))?;
        Ok((
            X_STAMP_HEADER.to_string(),
            BASE64_URL_SAFE_NO_PAD.encode(json),
        ))
    }
}

#[cfg(feature = "turnkey-p256")]
fn mismatch_message(expected: &str) -> String {
    format!(
        "API public key {expected} matches neither the secp256k1 nor the P-256 derivation of the private key"
    )
}

#[cfg(not(feature = "turnkey-p256"))]
fn mismatch_message(expected: &str) -> String {
    format!(
        "API public key {expected} does not match the secp256k1 derivation of the private key; if it is a P-256 key, build with the `turnkey-p256` feature"
    )
}
