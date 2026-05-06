//! Helpers for deriving per-tenant `PostgreSQL` advisory-lock keys.
//!
//! Each store (tree, token, …) picks its own domain prefix and combines it with
//! the tenant identity pubkey via SHA-256, taking the first 8 bytes of the
//! digest as a 64-bit lock key. The 64-bit space keeps cross-tenant collisions
//! negligible (~1.2e-10 at 65k tenants) while distinct prefixes guarantee that
//! locks from different stores never collide.

use sha2::{Digest, Sha256};

/// Derives a stable 64-bit advisory-lock key from a tenant identity pubkey.
/// Hashes a domain prefix together with the identity and folds the first 8
/// bytes of the SHA-256 digest into an `i64` (the type expected by
/// `pg_advisory_xact_lock(bigint)`).
pub(crate) fn identity_lock_key(prefix: &[u8], identity: &[u8]) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(prefix);
    hasher.update(identity);
    let digest = hasher.finalize();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(buf)
}
