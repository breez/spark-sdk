//! Turnkey request "stamper": signs each request body with the secp256k1 API
//! key and produces the `X-Stamp` header Turnkey requires.
//!
//! Turnkey supports both P-256 and secp256k1 API keys; we use secp256k1
//! (`SIGNATURE_SCHEME_TK_API_SECP256K1`) so the SDK's existing `bitcoin`/
//! `secp256k1` dependency covers it with no extra crypto crate. The configured
//! API keypair must therefore be created as secp256k1, not P-256.

use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{self, Message, PublicKey, Secp256k1, SecretKey};
use serde::Serialize;

use super::error::TurnkeyError;

const SIGNATURE_SCHEME_SECP256K1: &str = "SIGNATURE_SCHEME_TK_API_SECP256K1";
const X_STAMP_HEADER: &str = "X-Stamp";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiStamp {
    public_key: String,
    signature: String,
    scheme: String,
}

/// Signs request bodies with a Turnkey secp256k1 API key.
pub(crate) struct ApiKeyStamper {
    secret_key: SecretKey,
    public_key: PublicKey,
    secp: Secp256k1<secp256k1::All>,
}

impl ApiKeyStamper {
    /// Builds a stamper from the hex-encoded secp256k1 private key. If
    /// `expected_public_key_hex` is non-empty, the derived compressed public key
    /// is checked against it to catch a misconfigured keypair early.
    pub(crate) fn from_hex(
        private_key_hex: &str,
        expected_public_key_hex: &str,
    ) -> Result<Self, TurnkeyError> {
        let priv_bytes = hex::decode(private_key_hex)
            .map_err(|e| TurnkeyError::InvalidApiKey(format!("private key hex: {e}")))?;
        let secret_key = SecretKey::from_slice(&priv_bytes)
            .map_err(|e| TurnkeyError::InvalidApiKey(e.to_string()))?;
        let secp = Secp256k1::new();
        let public_key = secret_key.public_key(&secp);
        let stamper = Self {
            secret_key,
            public_key,
            secp,
        };
        if !expected_public_key_hex.is_empty() {
            let derived = stamper.public_key_hex();
            if !derived.eq_ignore_ascii_case(expected_public_key_hex) {
                return Err(TurnkeyError::InvalidApiKey(format!(
                    "public key mismatch: derived {derived}, configured {expected_public_key_hex}"
                )));
            }
        }
        Ok(stamper)
    }

    fn public_key_hex(&self) -> String {
        hex::encode(self.public_key.serialize())
    }

    /// Stamps `body`, returning the `(header_name, header_value)` to attach to
    /// the request. The value is a base64url-no-pad JSON object
    /// `{publicKey, signature, scheme}`; the signature is DER-encoded ECDSA over
    /// SHA-256 of the body.
    pub(crate) fn stamp(&self, body: &[u8]) -> Result<(String, String), TurnkeyError> {
        let digest = sha256::Hash::hash(body);
        let message = Message::from_digest(digest.to_byte_array());
        let sig = self.secp.sign_ecdsa(&message, &self.secret_key);
        let stamp = ApiStamp {
            public_key: self.public_key_hex(),
            signature: hex::encode(sig.serialize_der()),
            scheme: SIGNATURE_SCHEME_SECP256K1.to_string(),
        };
        let json =
            serde_json::to_vec(&stamp).map_err(|e| TurnkeyError::Serialize(e.to_string()))?;
        Ok((
            X_STAMP_HEADER.to_string(),
            BASE64_URL_SAFE_NO_PAD.encode(json),
        ))
    }
}
