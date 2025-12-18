use std::sync::Arc;

use crate::signer::BreezSigner;

#[derive(thiserror::Error, Debug)]
pub enum NostrSignerError {
    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),
    #[error("Zap receipt creation error: {0}")]
    ZapReceiptCreationError(String),
}

pub struct NostrSigner {
    signer: Arc<dyn BreezSigner>,
    pubkey: ::nostr::PublicKey,
}

impl NostrSigner {
    pub async fn new(signer: Arc<dyn BreezSigner>) -> Result<Self, NostrSignerError> {
        let pubkey_str = signer
            .nostr_pubkey()
            .await
            .map_err(|e| NostrSignerError::KeyDerivationError(e.to_string()))?;
        let pubkey = ::nostr::PublicKey::parse(&pubkey_str)
            .map_err(|e| NostrSignerError::KeyDerivationError(e.to_string()))?;
        Ok(Self { signer, pubkey })
    }

    pub fn nostr_pubkey(&self) -> String {
        self.pubkey.to_string()
    }

    pub async fn sign_event(
        &self,
        builder: ::nostr::event::EventBuilder,
    ) -> Result<::nostr::event::Event, NostrSignerError> {
        self.signer
            .sign_nostr_event(builder)
            .await
            .map_err(|e| NostrSignerError::KeyDerivationError(e.to_string()))
    }
}
