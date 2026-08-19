//! Validation of the SSP authentication challenge before it is signed.
//!
//! The SSP verifies its own HMAC over the challenge it issued, so the client
//! signs those bytes verbatim instead of re-encoding them from a decoded
//! message. That makes the challenge server-chosen input to the identity key,
//! accepted here only as an `ssp_authn.ProtectedChallenge` naming this wallet.
//!
//! How recent the challenge is stays the SSP's to enforce, since it expires its
//! own challenges: checking it against the local clock would reject nothing a
//! hostile server cannot sidestep by sending the current time, and would lock
//! out a device whose clock is unset.

use bitcoin::secp256k1::PublicKey;
use prost::Message;
use thiserror::Error;

use super::ssp_authn::ProtectedChallenge;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ChallengeError {
    #[error("malformed challenge: {0}")]
    Malformed(&'static str),

    #[error("challenge has no {0}")]
    Missing(&'static str),

    #[error("challenge is issued to another public key")]
    PublicKeyMismatch,
}

/// Accepts `bytes` as a challenge issued to `identity_public_key`, or explains
/// why it is not one. `bytes` are the decoded `protected_challenge`, exactly as
/// they will be signed.
pub(crate) fn validate(
    bytes: &[u8],
    identity_public_key: &PublicKey,
) -> Result<(), ChallengeError> {
    let protected = ProtectedChallenge::decode(bytes)
        .map_err(|_| ChallengeError::Malformed("not a protobuf message"))?;
    let challenge = protected
        .challenge
        .ok_or(ChallengeError::Missing("challenge"))?;
    if challenge.public_key.is_empty() {
        return Err(ChallengeError::Missing("public key"));
    }

    // Parsed and compared as a key rather than as bytes, so either encoding
    // verifies.
    let public_key = PublicKey::from_slice(&challenge.public_key)
        .map_err(|_| ChallengeError::Malformed("public key"))?;
    if public_key != *identity_public_key {
        return Err(ChallengeError::PublicKeyMismatch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssp::ssp_authn::Challenge;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// `protected_challenge` responses served by api.lightspark.com, the first
    /// requested for the secp256k1 generator point and the second for another
    /// key on the other endpoint. The schema is transcribed from one Lightspark
    /// shared rather than published, so these captures are what pin it.
    const LIVE: [(&str, &str); 2] = [
        (
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "0801524f080150e49e8cd406a201203cc9ed127c551fb4ea5fb97233115307c4fa9233ece3e6087aad7\
d95e01f9d7ef201210279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798a20120543f1936\
b8b6422fa7fdd92b8bb5aa8b0a31f0c7989675a2fec3a8c6487f42b7",
        ),
        (
            "03fff97bd5755eeea420453a14355235d382f6472f8568a18b2f057a1460297556",
            "0801524f080150bec28cd406a20120562fb45d2f8388acae10b882c86a08c6f4e4a93742f7f8568f83d\
d18a8024a32f2012103fff97bd5755eeea420453a14355235d382f6472f8568a18b2f057a1460297556a2012014b947ef\
eab6d6a3d34c2e19ee3bc94fbfd8893cf0b09c4d16df9d6d4549ebed",
        ),
    ];

    fn key(hex_encoded: &str) -> PublicKey {
        PublicKey::from_slice(&hex::decode(hex_encoded).unwrap()).unwrap()
    }

    /// A challenge issued to `public_key`, as the wire bytes it is signed as.
    fn issued_to(public_key: &[u8]) -> Vec<u8> {
        ProtectedChallenge {
            version: 1,
            challenge: Some(Challenge {
                version: 1,
                timestamp: 1_786_974_052,
                nonce: vec![7u8; 32],
                public_key: public_key.to_vec(),
            }),
            server_hmac: vec![9u8; 32],
        }
        .encode_to_vec()
    }

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = u8::try_from(value & 0x7f).unwrap();
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn bytes_field(number: u64, value: &[u8]) -> Vec<u8> {
        let mut out = varint(number << 3 | 2);
        out.extend(varint(u64::try_from(value.len()).unwrap()));
        out.extend_from_slice(value);
        out
    }

    /// Re-encoding a live response reproduces it byte for byte, so the schema
    /// accounts for every field the server sends: a transcription missing one
    /// would round-trip short.
    #[test_all]
    fn the_schema_accounts_for_every_field_of_a_live_challenge() {
        for (_, challenge) in LIVE {
            let bytes = hex::decode(challenge).unwrap();
            let decoded = ProtectedChallenge::decode(&bytes[..]).unwrap();
            assert_eq!(decoded.encode_to_vec(), bytes);
        }
    }

    #[test_all]
    fn accepts_a_live_challenge() {
        for (public_key, challenge) in LIVE {
            let bytes = hex::decode(challenge).unwrap();
            assert_eq!(validate(&bytes, &key(public_key)), Ok(()));
        }
    }

    #[test_all]
    fn rejects_a_challenge_issued_to_another_key() {
        let bytes = issued_to(&hex::decode(LIVE[0].0).unwrap());
        assert_eq!(
            validate(&bytes, &key(LIVE[1].0)),
            Err(ChallengeError::PublicKeyMismatch)
        );
    }

    /// The field is parsed as a key, not compared as bytes, so the encoding the
    /// server picks is not load-bearing. Worth pinning because the schema's own
    /// comment gets it wrong, so an encoding change is a plausible surprise.
    #[test_all]
    fn accepts_either_public_key_encoding() {
        let uncompressed = key(LIVE[0].0).serialize_uncompressed();
        assert_eq!(validate(&issued_to(&uncompressed), &key(LIVE[0].0)), Ok(()));
    }

    /// The property the whole module exists for: text of the sender's choosing
    /// is not a challenge, whatever it says.
    #[test_all]
    fn rejects_arbitrary_text() {
        for message in [
            &b"sign this please"[..],
            &b"v2\naction\nexample.com\nalice\n1786974052"[..],
        ] {
            assert!(validate(message, &key(LIVE[0].0)).is_err());
        }
    }

    /// Fields the schema does not declare pass through wherever they sit, so a
    /// field the SSP adds later does not fail authentication, and neither does a
    /// serializer that emits out of tag order.
    #[test_all]
    fn tolerates_unknown_fields() {
        let live = hex::decode(LIVE[0].1).unwrap();

        let mut trailing = live.clone();
        trailing.extend(bytes_field(40, b"added later"));
        assert_eq!(validate(&trailing, &key(LIVE[0].0)), Ok(()));

        let mut leading = bytes_field(12, b"added later");
        leading.extend(live);
        assert_eq!(validate(&leading, &key(LIVE[0].0)), Ok(()));
    }

    #[test_all]
    fn rejects_a_missing_field() {
        assert_eq!(
            validate(&issued_to(&[]), &key(LIVE[0].0)),
            Err(ChallengeError::Missing("public key"))
        );

        let no_challenge = ProtectedChallenge {
            version: 1,
            challenge: None,
            server_hmac: vec![9u8; 32],
        }
        .encode_to_vec();
        assert_eq!(
            validate(&no_challenge, &key(LIVE[0].0)),
            Err(ChallengeError::Missing("challenge"))
        );
    }

    #[test_all]
    fn rejects_a_public_key_that_is_not_one() {
        assert_eq!(
            validate(&issued_to(&[0u8; 33]), &key(LIVE[0].0)),
            Err(ChallengeError::Malformed("public key"))
        );
    }
}
