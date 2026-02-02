use bitcoin::hashes::{Hash, sha256};
use breez_sdk_spark::seedless_restore::{PasskeyPrfError, PasskeyPrfProvider};
use challenge_response::ChallengeResponse;
use challenge_response::config::{Config, Mode, Slot};

/// `YubiKey` HMAC challenge-response implementation of `PasskeyPrfProvider`.
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
    /// Returns `PasskeyPrfError::CredentialNotFound` if no `YubiKey` is connected.
    pub fn new() -> Result<Self, PasskeyPrfError> {
        let mut cr = ChallengeResponse::new()
            .map_err(|e| PasskeyPrfError::Generic(format!("Failed to init YubiKey: {e}")))?;
        cr.find_device()
            .map_err(|_| PasskeyPrfError::CredentialNotFound)?;
        Ok(Self)
    }
}

#[async_trait::async_trait]
impl PasskeyPrfProvider for YubiKeyPrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        eprintln!("Touch your YubiKey (if configured)...");

        tokio::task::spawn_blocking(move || {
            let mut cr = ChallengeResponse::new()
                .map_err(|e| PasskeyPrfError::Generic(format!("YubiKey init failed: {e}")))?;
            let device = cr
                .find_device()
                .map_err(|_| PasskeyPrfError::CredentialNotFound)?;

            let config = Config::new_from(device)
                .set_mode(Mode::Sha1)
                .set_slot(Slot::Slot2);

            let challenge = salt.as_bytes();
            let hmac_result = cr.challenge_response_hmac(challenge, config).map_err(|e| {
                let msg = format!("{e}");
                if msg.contains("Wrong CRC") {
                    PasskeyPrfError::PrfEvaluationFailed(
                        "YubiKey Slot 2 is not configured for HMAC challenge-response. \
                             Program it with: ykman otp chalresp -g 2"
                            .to_string(),
                    )
                } else {
                    PasskeyPrfError::PrfEvaluationFailed(format!("HMAC failed: {e}"))
                }
            })?;

            // Expand 20-byte HMAC-SHA1 output to 32 bytes via SHA256
            let hash = sha256::Hash::hash(&hmac_result);

            Ok(hash.to_byte_array().to_vec())
        })
        .await
        .map_err(|e| PasskeyPrfError::Generic(format!("Task join error: {e}")))?
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        let mut cr = ChallengeResponse::new()
            .map_err(|_| PasskeyPrfError::Generic("YubiKey init failed".into()))?;
        Ok(cr.find_device().is_ok())
    }
}
