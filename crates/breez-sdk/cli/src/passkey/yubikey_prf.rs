use bitcoin::hashes::{Hash, sha256};
use breez_sdk_spark::passkey::{
    DeriveSeedsOutput, DeriveSeedsRequest, PrfProvider, PrfProviderError,
};
use challenge_response::ChallengeResponse;
use challenge_response::config::{Config, Mode, Slot};

/// `YubiKey` HMAC challenge-response implementation of `PrfProvider`.
///
/// Uses HMAC-SHA1 from `YubiKey` Slot 2, then expands to 32 bytes via SHA256.
/// The expansion is performed using `SHA256(hmac_output)` for cross-implementation
/// portability.
///
/// # Security Notes
///
/// - The 20-byte HMAC-SHA1 output is expanded to 32 bytes using SHA256
/// - Different salts produce different outputs (deterministically)
/// - If Slot 2 was programmed with `-t`, each derivation requires physical touch
pub struct YubiKeyPrfProvider;

impl YubiKeyPrfProvider {
    /// Create a new `YubiKeyPrfProvider`.
    ///
    /// Verifies that a `YubiKey` is connected during construction.
    ///
    /// # Errors
    /// Returns `PrfProviderError::CredentialNotFound` if no `YubiKey` is connected.
    pub fn new() -> Result<Self, PrfProviderError> {
        let mut cr = ChallengeResponse::new()
            .map_err(|e| PrfProviderError::Generic(format!("Failed to init YubiKey: {e}")))?;
        cr.find_device().map_err(|_| {
            PrfProviderError::CredentialNotFound("No YubiKey connected".to_string())
        })?;
        Ok(Self)
    }
}

impl YubiKeyPrfProvider {
    fn derive_one_blocking(salt: &str) -> Result<Vec<u8>, PrfProviderError> {
        let mut cr = ChallengeResponse::new()
            .map_err(|e| PrfProviderError::Generic(format!("YubiKey init failed: {e}")))?;
        let device = cr.find_device().map_err(|_| {
            PrfProviderError::CredentialNotFound("No YubiKey connected".to_string())
        })?;

        let config = Config::new_from(device)
            .set_mode(Mode::Sha1)
            .set_slot(Slot::Slot2);

        let hmac_result = cr
            .challenge_response_hmac(salt.as_bytes(), config)
            .map_err(|e| {
                let msg = format!("{e}");
                if msg.contains("Wrong CRC") {
                    PrfProviderError::PrfEvaluationFailed(
                        "YubiKey Slot 2 is not configured for HMAC challenge-response. \
                         Program it with: ykman otp chalresp -g 2"
                            .to_string(),
                    )
                } else {
                    PrfProviderError::PrfEvaluationFailed(format!("HMAC failed: {e}"))
                }
            })?;

        // Expand 20-byte HMAC-SHA1 output to 32 bytes via SHA256
        Ok(sha256::Hash::hash(&hmac_result).to_byte_array().to_vec())
    }
}

#[async_trait::async_trait]
impl PrfProvider for YubiKeyPrfProvider {
    async fn derive_seeds(
        &self,
        request: DeriveSeedsRequest,
    ) -> Result<DeriveSeedsOutput, PrfProviderError> {
        // YubiKey HMAC-SHA1 challenge-response has no platform picker;
        // the per-call allow-list and immediate-mediation hint are
        // no-ops here, and there is no credential ID to surface.
        let mut seeds = Vec::with_capacity(request.salts.len());
        for salt in request.salts {
            eprintln!("Touch your YubiKey (if configured)...");
            let seed = tokio::task::spawn_blocking(move || Self::derive_one_blocking(&salt))
                .await
                .map_err(|e| PrfProviderError::Generic(format!("Task join error: {e}")))??;
            seeds.push(seed);
        }
        Ok(DeriveSeedsOutput {
            seeds,
            credential_id: None,
        })
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        let mut cr = ChallengeResponse::new()
            .map_err(|_| PrfProviderError::Generic("YubiKey init failed".into()))?;
        Ok(cr.find_device().is_ok())
    }
}
