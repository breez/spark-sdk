use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use bitcoin::hashes::{Hash as _, HashEngine as _, Hmac, HmacEngine, sha256};
use bitcoin::secp256k1::{All, Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use prost::Message as _;
use thiserror::Error;

mod ssp_authn {
    tonic::include_proto!("ssp_authn");
}
use ssp_authn::{Challenge, ProtectedChallenge};

const CHALLENGE_TTL_SECS: i64 = 300;
const SESSION_TTL_SECS: i64 = 24 * 60 * 60;
const CLOCK_SKEW_SECS: i64 = 60;

const SESSION_TOKEN_LEN: usize = 33 + 8 + 32; // pubkey || expiry(i64 be) || hmac

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("malformed request: {0}")]
    Malformed(&'static str),
    #[error("challenge was not issued by this server")]
    BadChallengeHmac,
    #[error("challenge has expired")]
    ChallengeExpired,
    #[error("challenge is issued to another public key")]
    PublicKeyMismatch,
    #[error("challenge signature is invalid")]
    BadSignature,
    #[error("session token is invalid")]
    BadSession,
    #[error("session token has expired")]
    SessionExpired,
}

pub struct AuthService {
    challenge_key: [u8; 32],
    session_key: [u8; 32],
    secp: Secp256k1<All>,
}

impl AuthService {
    /// Derives separate keys for challenges and session tokens, so an HMAC made
    /// for one never verifies as the other.
    pub fn from_seed(seed: &[u8]) -> Self {
        Self {
            challenge_key: subkey(seed, b"sspd-auth-challenge-v1"),
            session_key: subkey(seed, b"sspd-auth-session-v1"),
            secp: Secp256k1::new(),
        }
    }

    pub fn issue_challenge(&self, public_key_hex: &str, now: i64) -> Result<String, AuthError> {
        let public_key = parse_pubkey_hex(public_key_hex)?;
        let mut nonce_rng = bitcoin::secp256k1::rand::thread_rng();
        let nonce = SecretKey::new(&mut nonce_rng).secret_bytes().to_vec();

        let challenge = Challenge {
            version: 1,
            timestamp: now,
            nonce,
            public_key: public_key.serialize().to_vec(),
        };
        let server_hmac = hmac(&self.challenge_key, &challenge.encode_to_vec()).to_vec();
        let protected = ProtectedChallenge {
            version: 1,
            challenge: Some(challenge),
            server_hmac,
        };
        Ok(B64.encode(protected.encode_to_vec()))
    }

    /// Returns a session token and its expiry time in Unix seconds.
    pub fn verify_challenge(
        &self,
        protected_challenge: &str,
        signature: &str,
        identity_public_key: &str,
        now: i64,
    ) -> Result<(String, i64), AuthError> {
        let protected_bytes = B64
            .decode(protected_challenge)
            .map_err(|_| AuthError::Malformed("protected_challenge is not base64url"))?;
        let protected = ProtectedChallenge::decode(&protected_bytes[..])
            .map_err(|_| AuthError::Malformed("protected_challenge is not a protobuf message"))?;
        let challenge = protected
            .challenge
            .ok_or(AuthError::Malformed("challenge missing"))?;

        let expected = hmac(&self.challenge_key, &challenge.encode_to_vec());
        if !constant_time_eq(&protected.server_hmac, expected.as_ref()) {
            return Err(AuthError::BadChallengeHmac);
        }

        if now.saturating_sub(challenge.timestamp) > CHALLENGE_TTL_SECS
            || challenge.timestamp.saturating_sub(now) > CLOCK_SKEW_SECS
        {
            return Err(AuthError::ChallengeExpired);
        }

        let identity = parse_pubkey_hex(identity_public_key)?;
        let challenge_key = PublicKey::from_slice(&challenge.public_key)
            .map_err(|_| AuthError::Malformed("challenge public key"))?;
        if challenge_key != identity {
            return Err(AuthError::PublicKeyMismatch);
        }

        let signature = B64
            .decode(signature)
            .map_err(|_| AuthError::Malformed("signature is not base64url"))?;
        let signature =
            Signature::from_der(&signature).map_err(|_| AuthError::Malformed("signature DER"))?;
        let digest = sha256::Hash::hash(&protected_bytes).to_byte_array();
        let message = Message::from_digest(digest);
        self.secp
            .verify_ecdsa(&message, &signature, &identity)
            .map_err(|_| AuthError::BadSignature)?;

        let expiry = now.saturating_add(SESSION_TTL_SECS);
        Ok((self.mint_session(&identity, expiry), expiry))
    }

    pub fn validate_session(&self, token: &str, now: i64) -> Result<PublicKey, AuthError> {
        let raw = B64.decode(token).map_err(|_| AuthError::BadSession)?;
        if raw.len() != SESSION_TOKEN_LEN {
            return Err(AuthError::BadSession);
        }
        let (payload, mac) = raw.split_at_checked(33 + 8).ok_or(AuthError::BadSession)?;
        let expected = hmac(&self.session_key, payload);
        if !constant_time_eq(mac, expected.as_ref()) {
            return Err(AuthError::BadSession);
        }
        let (identity, expiry) = payload.split_at_checked(33).ok_or(AuthError::BadSession)?;
        let expiry = i64::from_be_bytes(expiry.try_into().map_err(|_| AuthError::BadSession)?);
        if expiry <= now {
            return Err(AuthError::SessionExpired);
        }
        PublicKey::from_slice(identity).map_err(|_| AuthError::BadSession)
    }

    fn mint_session(&self, identity: &PublicKey, expiry: i64) -> String {
        let mut payload = Vec::with_capacity(SESSION_TOKEN_LEN);
        payload.extend_from_slice(&identity.serialize());
        payload.extend_from_slice(&expiry.to_be_bytes());
        let mac = hmac(&self.session_key, &payload);
        payload.extend_from_slice(mac.as_ref());
        B64.encode(payload)
    }
}

fn subkey(seed: &[u8], domain: &[u8]) -> [u8; 32] {
    let mut engine = HmacEngine::<sha256::Hash>::new(seed);
    engine.input(domain);
    Hmac::<sha256::Hash>::from_engine(engine).to_byte_array()
}

fn hmac(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut engine = HmacEngine::<sha256::Hash>::new(key);
    engine.input(data);
    Hmac::<sha256::Hash>::from_engine(engine).to_byte_array()
}

fn parse_pubkey_hex(hex_str: &str) -> Result<PublicKey, AuthError> {
    let bytes = hex::decode(hex_str).map_err(|_| AuthError::Malformed("public key hex"))?;
    PublicKey::from_slice(&bytes).map_err(|_| AuthError::Malformed("public key"))
}

/// Does not stop at the first differing byte, so that timing does not reveal how
/// much of an HMAC matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};

    const NOW: i64 = 1_786_974_052;

    fn keypair() -> (SecretKey, PublicKey) {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        (sk, sk.public_key(&secp))
    }

    /// Signs as the SDK client does.
    fn sign(protected_challenge: &str, sk: &SecretKey) -> String {
        let bytes = B64.decode(protected_challenge).unwrap();
        let digest = sha256::Hash::hash(&bytes).to_byte_array();
        let secp = Secp256k1::new();
        let sig = secp.sign_ecdsa(&Message::from_digest(digest), sk);
        B64.encode(sig.serialize_der())
    }

    #[test]
    fn round_trips_a_valid_handshake() {
        let auth = AuthService::from_seed(b"seed");
        let (sk, pk) = keypair();
        let pk_hex = hex::encode(pk.serialize());

        let challenge = auth.issue_challenge(&pk_hex, NOW).unwrap();
        let signature = sign(&challenge, &sk);
        let (token, valid_until) = auth
            .verify_challenge(&challenge, &signature, &pk_hex, NOW)
            .unwrap();
        assert_eq!(valid_until, NOW + SESSION_TTL_SECS);
        assert_eq!(auth.validate_session(&token, NOW).unwrap(), pk);
    }

    #[test]
    fn rejects_a_challenge_from_another_server() {
        let issuer = AuthService::from_seed(b"issuer-seed");
        let verifier = AuthService::from_seed(b"other-seed");
        let (sk, pk) = keypair();
        let pk_hex = hex::encode(pk.serialize());

        let challenge = issuer.issue_challenge(&pk_hex, NOW).unwrap();
        let signature = sign(&challenge, &sk);
        assert_eq!(
            verifier.verify_challenge(&challenge, &signature, &pk_hex, NOW),
            Err(AuthError::BadChallengeHmac)
        );
    }

    #[test]
    fn rejects_an_expired_challenge() {
        let auth = AuthService::from_seed(b"seed");
        let (sk, pk) = keypair();
        let pk_hex = hex::encode(pk.serialize());

        let challenge = auth.issue_challenge(&pk_hex, NOW).unwrap();
        let signature = sign(&challenge, &sk);
        assert_eq!(
            auth.verify_challenge(
                &challenge,
                &signature,
                &pk_hex,
                NOW + CHALLENGE_TTL_SECS + 1
            ),
            Err(AuthError::ChallengeExpired)
        );
    }

    #[test]
    fn rejects_a_signature_by_the_wrong_key() {
        let auth = AuthService::from_seed(b"seed");
        let (_sk, pk) = keypair();
        let pk_hex = hex::encode(pk.serialize());
        let wrong_sk = SecretKey::from_slice(&[0x22; 32]).unwrap();

        let challenge = auth.issue_challenge(&pk_hex, NOW).unwrap();
        let signature = sign(&challenge, &wrong_sk);
        assert_eq!(
            auth.verify_challenge(&challenge, &signature, &pk_hex, NOW),
            Err(AuthError::BadSignature)
        );
    }

    #[test]
    fn rejects_verifying_under_a_different_identity_than_the_challenge() {
        let auth = AuthService::from_seed(b"seed");
        let (sk, pk) = keypair();
        let pk_hex = hex::encode(pk.serialize());
        let other = Keypair::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[0x33; 32]).unwrap(),
        );
        let other_hex = hex::encode(other.public_key().serialize());

        let challenge = auth.issue_challenge(&pk_hex, NOW).unwrap();
        let signature = sign(&challenge, &sk);
        assert_eq!(
            auth.verify_challenge(&challenge, &signature, &other_hex, NOW),
            Err(AuthError::PublicKeyMismatch)
        );
    }

    #[test]
    fn rejects_a_tampered_or_expired_session() {
        let auth = AuthService::from_seed(b"seed");
        let (sk, pk) = keypair();
        let pk_hex = hex::encode(pk.serialize());
        let challenge = auth.issue_challenge(&pk_hex, NOW).unwrap();
        let (token, _) = auth
            .verify_challenge(&challenge, &sign(&challenge, &sk), &pk_hex, NOW)
            .unwrap();

        assert_eq!(
            auth.validate_session(&token, NOW + SESSION_TTL_SECS + 1),
            Err(AuthError::SessionExpired)
        );
        let mut raw = B64.decode(&token).unwrap();
        *raw.last_mut().unwrap() ^= 0x01;
        assert_eq!(
            auth.validate_session(&B64.encode(raw), NOW),
            Err(AuthError::BadSession)
        );
    }
}
