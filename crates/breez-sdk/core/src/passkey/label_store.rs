//! Pluggable label storage. The default Nostr-backed implementation
//! lives in `nostr_client`; integrators can supply their own (e.g.
//! server-mediated, in-memory for tests) to opt out of Nostr.

use super::error::PasskeyError;

/// Opaque identity for label-store operations. Wraps the Nostr
/// keypair derived from the passkey's account-master PRF output.
/// Custom [`LabelStore`] implementors can call
/// [`Identity::public_key_bytes`] for a stable, non-secret user
/// identifier; the default Nostr-backed implementation in this
/// crate uses the underlying keypair directly via a private accessor.
#[derive(Debug, Clone)]
pub struct Identity {
    pub(crate) keys: nostr::Keys,
}

impl Identity {
    /// Compressed secp256k1 public key bytes (33 bytes). Stable per
    /// passkey + RP combination. Not secret; safe to send to a
    /// server backend as a user identifier.
    #[must_use]
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.keys.public_key().to_bytes().to_vec()
    }
}

/// Pluggable backend for label storage and discovery. The default
/// is Nostr-backed (relays); host integrators can implement this
/// trait against their own server, an in-memory store for tests, or
/// any other backend.
#[macros::async_trait]
pub trait LabelStore: Send + Sync {
    /// Discover all labels published by `identity`.
    async fn list_labels(&self, identity: &Identity) -> Result<Vec<String>, PasskeyError>;

    /// Idempotently ensure `label` is published for `identity`. If
    /// already present, no-op. Implementors should collapse
    /// "exists check" + "write" into one round-trip when their
    /// transport supports it.
    async fn ensure_label_published(
        &self,
        identity: &Identity,
        label: &str,
    ) -> Result<(), PasskeyError>;

    /// Publish `label` unconditionally. Used by callers that
    /// already know the label is new.
    async fn store_label(&self, identity: &Identity, label: &str) -> Result<(), PasskeyError>;
}
