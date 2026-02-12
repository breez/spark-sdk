use std::sync::Arc;

use crate::signer::nostr::NostrSigner;

#[derive(thiserror::Error, Debug)]
pub enum NostrError {
    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),
    #[error("Zap receipt creation error: {0}")]
    ZapReceiptCreationError(String),
}

pub struct NostrClient {
    signer: Arc<NostrSigner>,
}

impl NostrClient {
    pub fn new(signer: Arc<NostrSigner>) -> Self {
        NostrClient { signer }
    }

    pub fn nostr_pubkey(&self) -> String {
        self.signer.nostr_pubkey()
    }
}
