//! Helpers for deriving per-tenant `MySQL` named-lock keys.
//!
//! Each store (tree, token, …) picks its own domain prefix and combines it with
//! the tenant identity pubkey via SHA-256. The hex-encoded first 8 bytes of the
//! digest are used as the `GET_LOCK` name suffix. Distinct prefixes guarantee
//! that locks from different stores never collide; the 64-bit space keeps
//! cross-tenant collisions negligible (~1.2e-10 at 65k tenants).

use bitcoin::hashes::{Hash, HashEngine, sha256};

/// Derives a stable per-tenant lock name from a tenant identity pubkey.
/// Hashes a domain prefix together with the identity and folds the first 8
/// bytes of the SHA-256 digest into a hex string. `MySQL` `GET_LOCK` requires
/// a string name (max 64 chars), so we hex-encode rather than use raw bytes.
pub(crate) fn identity_lock_name(prefix: &str, identity: &[u8]) -> String {
    let mut engine = sha256::Hash::engine();
    engine.input(prefix.as_bytes());
    engine.input(identity);
    let digest = sha256::Hash::from_engine(engine);
    format!("{prefix}{}", hex::encode(&digest.as_byte_array()[..8]))
}
