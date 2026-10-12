use std::collections::VecDeque;
use std::sync::Mutex;

use frost_secp256k1_tr::round1::{SigningCommitments, SigningNonces};

use crate::signer::SignerError;

/// Most unconsumed nonces held before the oldest are evicted. An entry is only
/// left pending by a flow that commits and signs in separate calls, so this
/// sits far above steady-state use. Eviction fails the stale flow closed with
/// [`SignerError::UnknownNonceCommitment`].
const MAX_PENDING_NONCES: usize = 1024;

/// Single-use store of the FROST nonces a signer has issued.
///
/// Signing twice under one nonce leaks the signing share, so a nonce may be
/// signed with at most once. Nonces are held here rather than taken from the
/// caller, so a caller cannot choose them or present one twice.
///
/// Share one store across signers when the commit and the sign happen on
/// different instances. Nonces carry no key material, so sharing is safe even
/// between signers on different masters.
#[derive(Default)]
pub struct NonceStore {
    /// Pending nonces, oldest first. Bounded and scanned linearly: one entry is
    /// removed per signature, so the scan never competes with the signing.
    pending: Mutex<VecDeque<SigningNonces>>,
}

impl NonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `nonces` as issued and unconsumed.
    pub(crate) fn insert(&self, nonces: SigningNonces) -> Result<(), SignerError> {
        let mut pending = self.lock()?;
        pending.push_back(nonces);
        while pending.len() > MAX_PENDING_NONCES {
            pending.pop_front();
        }
        Ok(())
    }

    /// Consumes the nonces behind `commitments`, so a second call for the same
    /// commitment fails. Errors with [`SignerError::UnknownNonceCommitment`]
    /// when the commitment was not issued here, was already signed with, or was
    /// evicted.
    pub(crate) fn take(
        &self,
        commitments: &SigningCommitments,
    ) -> Result<SigningNonces, SignerError> {
        let mut pending = self.lock()?;
        let at = pending
            .iter()
            .position(|nonces| nonces.commitments() == commitments)
            .ok_or(SignerError::UnknownNonceCommitment)?;
        Ok(pending
            .remove(at)
            .expect("position returned an in-range index"))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, VecDeque<SigningNonces>>, SignerError> {
        self.pending
            .lock()
            .map_err(|_| SignerError::Generic("nonce store lock poisoned".to_string()))
    }
}
