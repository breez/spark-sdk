//! Canonical messages the identity key signs to authorize an LNURL server
//! request. Shared by the server and every client so both sides build the
//! exact same bytes.
//!
//! A message names the version, the route, the domain it applies to, and the
//! request it authorizes, joined by LF. Field order is fixed and messages are
//! only ever compared, never parsed, so the sole hazard is two different field
//! tuples building identical bytes. LF makes that structural: HTTP forbids LF
//! in a header value so no resolved domain can carry one,
//! `USERNAME_VALIDATION_REGEX` excludes it, hashes and pubkeys are hex,
//! timestamps are digits, and the one free-form field (the description) enters
//! hashed.
//!
//! The `golden_vectors` test below is the byte-level spec a third-party client
//! implements against: it is machine-checked, so unlike prose it cannot drift
//! from what the server verifies.
//!
//! A signature covers the whole message, so it authorizes one request on one
//! domain and nothing else. [`RESERVED_NAMESPACE`] keeps these messages to
//! themselves.

use sha2::{Digest, Sha256};

/// Namespace every message in this module begins with, reserved for the LNURL
/// server across all versions.
///
/// The SDK's public message-signing API refuses it, so these messages stay the
/// SDK's own to produce.
pub const RESERVED_NAMESPACE: &str = "breez-lnurl:";

/// First field of every message: the namespace and the version, so an unknown
/// version is rejected outright rather than falling through to an older
/// interpretation.
const VERSION: &str = "breez-lnurl:v2";

/// Field separator. See the module docs for why it is LF.
const SEPARATOR: char = '\n';

/// How far a message's `timestamp` may sit from the verifier's clock, in either
/// direction.
///
/// Lives here, with the builders whose output it bounds, because the two sides
/// have to agree: the server refuses a message outside this window, and a
/// client that applies a wider one just sends requests that are rejected. It
/// also sets how long the server retains the claim over a spent message, which
/// is what lets a timestamped statement be forgotten at all, so widening it
/// widens retention by the same amount.
pub const VALIDITY_SECS: u64 = 600;

/// `sha256` hex of the exact description bytes the server stores.
///
/// The description is never trimmed or normalized on either side, so the two
/// cannot start now without every signature over it failing to verify.
#[must_use]
pub fn description_hash(description: &str) -> String {
    let digest = Sha256::digest(description.as_bytes());
    let mut hex = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        // Lowercase hex, two digits per byte.
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    hex
}

fn join(fields: &[&str]) -> String {
    fields.join(&SEPARATOR.to_string())
}

/// `POST /lnurlpay/{pubkey}`: claim `username` on `domain`.
#[must_use]
pub fn register(domain: &str, username: &str, description: &str, timestamp: u64) -> String {
    join(&[
        VERSION,
        "register",
        domain,
        username,
        &description_hash(description),
        &timestamp.to_string(),
    ])
}

/// `DELETE /lnurlpay/{pubkey}`: give up `username` on `domain`.
#[must_use]
pub fn unregister(domain: &str, username: &str, timestamp: u64) -> String {
    join(&[
        VERSION,
        "unregister",
        domain,
        username,
        &timestamp.to_string(),
    ])
}

/// `POST /lnurlpay/{pubkey}/recover`: read back the address `pubkey` holds on
/// `domain`.
#[must_use]
pub fn recover(domain: &str, pubkey: &str, timestamp: u64) -> String {
    join(&[VERSION, "recover", domain, pubkey, &timestamp.to_string()])
}

/// `GET /lnurlpay/{pubkey}/metadata`: read `pubkey`'s payment metadata.
///
/// Commits to no pagination parameter: binding them would couple client
/// formatting to server re-serialization, and paging only ever selects among
/// the caller's own rows.
#[must_use]
pub fn metadata(domain: &str, pubkey: &str, timestamp: u64) -> String {
    join(&[VERSION, "metadata", domain, pubkey, &timestamp.to_string()])
}

/// `POST /lnurlpay/{to_pubkey}/transfer`, signed by the current owner A.
///
/// Does not commit to the description: A is handing the username over, so the
/// `text/plain` metadata payers see afterwards is the transferee's to choose.
#[must_use]
pub fn transfer_from(
    domain: &str,
    username: &str,
    from_pubkey: &str,
    to_pubkey: &str,
    timestamp: u64,
) -> String {
    join(&[
        VERSION,
        "transfer-from",
        domain,
        username,
        from_pubkey,
        to_pubkey,
        &timestamp.to_string(),
    ])
}

/// `POST /lnurlpay/{to_pubkey}/transfer`, signed by the transferee B.
///
/// The role tag distinguishes the two signatures, so neither stands in for the
/// other, and B commits to the description because B is the one choosing it.
#[must_use]
pub fn transfer_to(
    domain: &str,
    username: &str,
    from_pubkey: &str,
    to_pubkey: &str,
    description: &str,
    timestamp: u64,
) -> String {
    join(&[
        VERSION,
        "transfer-to",
        domain,
        username,
        from_pubkey,
        to_pubkey,
        &description_hash(description),
        &timestamp.to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN: &str = "lnurl.example.com";
    const ALICE: &str = "02a1633cafcc01ebfb6d78e39f687a1f0995c62fc95f51ead10a02ee0be551b5dc";
    const BOB: &str = "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    /// Byte-level vectors pinning the wire format. This is the protocol: a
    /// refactor that changes them changes what every deployed server verifies,
    /// so it is a wire break rather than a test to update.
    #[test]
    fn golden_vectors() {
        assert_eq!(
            register(DOMAIN, "alice", "Pay to alice", 1_700_000_000),
            concat!(
                "breez-lnurl:v2\nregister\nlnurl.example.com\nalice\n",
                "0261d8b11c7eba9f71ccf5180df58416d3b2d19ba917caf57ba4bacead3ce2c2",
                "\n1700000000"
            )
        );
        assert_eq!(
            unregister(DOMAIN, "alice", 1_700_000_000),
            "breez-lnurl:v2\nunregister\nlnurl.example.com\nalice\n1700000000"
        );
        assert_eq!(
            recover(DOMAIN, ALICE, 1_700_000_000),
            format!("breez-lnurl:v2\nrecover\nlnurl.example.com\n{ALICE}\n1700000000")
        );
        assert_eq!(
            metadata(DOMAIN, ALICE, 1_700_000_000),
            format!("breez-lnurl:v2\nmetadata\nlnurl.example.com\n{ALICE}\n1700000000")
        );
        assert_eq!(
            transfer_from(DOMAIN, "alice", ALICE, BOB, 1_700_000_000),
            format!(
                "breez-lnurl:v2\ntransfer-from\nlnurl.example.com\nalice\n{ALICE}\n{BOB}\n1700000000"
            )
        );
        assert_eq!(
            transfer_to(DOMAIN, "alice", ALICE, BOB, "Pay to alice", 1_700_000_000),
            format!(
                "breez-lnurl:v2\ntransfer-to\nlnurl.example.com\nalice\n{ALICE}\n{BOB}\n\
                 0261d8b11c7eba9f71ccf5180df58416d3b2d19ba917caf57ba4bacead3ce2c2\n1700000000"
            )
        );
    }

    #[test]
    fn description_hash_is_lowercase_sha256_hex() {
        assert_eq!(
            description_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            description_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// The description enters hashed, so a description carrying the separator
    /// cannot shift the fields that follow it.
    #[test]
    fn a_description_containing_the_separator_stays_one_field() {
        let sneaky = "x\n1700000000";
        let message = register(DOMAIN, "alice", sneaky, 1_700_000_000);
        assert_eq!(message.matches(SEPARATOR).count(), 5);
        assert!(!message.contains(sneaky));
    }

    /// Every message leads with the version field, so a future version is
    /// distinguishable from this one at the first byte rather than by parsing.
    #[test]
    fn every_message_leads_with_the_version() {
        for message in [
            register(DOMAIN, "alice", "d", 1),
            unregister(DOMAIN, "alice", 1),
            recover(DOMAIN, ALICE, 1),
            metadata(DOMAIN, ALICE, 1),
            transfer_from(DOMAIN, "alice", ALICE, BOB, 1),
            transfer_to(DOMAIN, "alice", ALICE, BOB, "d", 1),
        ] {
            assert!(message.starts_with(VERSION), "{message}");
        }
    }

    /// The guard in the SDK's public signing API keys off the namespace, not the
    /// version, so every version has to sit inside it.
    #[test]
    fn the_version_sits_in_the_reserved_namespace() {
        assert!(VERSION.starts_with(RESERVED_NAMESPACE));
    }

    /// Distinct routes over the same tuple build distinct messages, so a
    /// signature is only ever valid for the route it was made for.
    #[test]
    fn routes_over_the_same_tuple_build_distinct_messages() {
        let messages = [
            recover(DOMAIN, ALICE, 1),
            metadata(DOMAIN, ALICE, 1),
            unregister(DOMAIN, ALICE, 1),
            transfer_from(DOMAIN, "alice", ALICE, BOB, 1),
            transfer_to(DOMAIN, "alice", ALICE, BOB, "d", 1),
            // The reversed pair is a different transfer, not the same one.
            transfer_from(DOMAIN, "alice", BOB, ALICE, 1),
        ];
        for (i, a) in messages.iter().enumerate() {
            for b in &messages[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// A domain is one field, so no username can be crafted to make a message
    /// for one domain read as a message for another.
    #[test]
    fn a_username_cannot_impersonate_another_domain() {
        assert_ne!(
            unregister("a.com", "alice", 1),
            unregister("b.com", "alice", 1)
        );
        assert_ne!(
            unregister("a.com", "x", 1),
            unregister("a.com\nx", "alice", 1),
        );
    }
}
