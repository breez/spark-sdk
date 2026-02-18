use crate::{Payment, PaymentDetails, signer::nostr::NostrSigner};
use nostr::JsonUtil;

#[derive(thiserror::Error, Debug)]
pub enum NostrError {
    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),
    #[error("Zap receipt creation error: {0}")]
    ZapReceiptCreationError(String),
}

use std::sync::Arc;

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

    pub async fn create_zap_receipt(
        &self,
        zap_request: &str,
        payment: &Payment,
    ) -> Result<String, NostrError> {
        // Extract invoice and preimage from payment details
        let Some(PaymentDetails::Lightning {
            invoice,
            htlc_details,
            ..
        }) = &payment.details
        else {
            return Err(NostrError::ZapReceiptCreationError(
                "Payment is not a lightning payment".to_string(),
            ));
        };

        let builder = lnurl_models::nostr::create_zap_receipt(
            zap_request,
            invoice,
            htlc_details.preimage.clone(),
        )
        .map_err(NostrError::ZapReceiptCreationError)?;

        self.signer
            .sign_event(builder)
            .await
            .map_err(|e| NostrError::KeyDerivationError(e.to_string()))?
            .try_as_json()
            .map_err(|e| {
                NostrError::ZapReceiptCreationError(format!("Failed to serialize zap receipt: {e}"))
            })
    }
}
