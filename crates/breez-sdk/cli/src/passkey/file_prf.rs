use std::fs;
use std::path::PathBuf;

use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use breez_sdk_spark::passkey::{
    DeriveSeedsOutput, DeriveSeedsRequest, PrfProvider, PrfProviderError,
};
use rand::{RngCore, thread_rng};

/// File name for the seed restore secret.
const SECRET_FILE_NAME: &str = "seedless-restore-secret";

/// File-based implementation of `PrfProvider`.
///
/// Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
/// randomly on first use and persisted to disk.
///
/// # Security Notes
///
/// - The secret file should be protected with appropriate file permissions
/// - This is less secure than hardware-backed solutions like `YubiKey`
/// - Suitable for development/testing or when hardware keys are unavailable
pub struct FilePrfProvider {
    secret: [u8; 32],
}

impl FilePrfProvider {
    /// Create a new `FilePrfProvider` using a secret from the specified data directory.
    ///
    /// If the secret file doesn't exist, a random 32-byte secret is generated and saved.
    ///
    /// # Arguments
    /// * `data_dir` - The data directory where the secret file is stored
    ///
    /// # Errors
    /// Returns `PrfProviderError::Generic` if file operations fail.
    pub fn new(data_dir: &PathBuf) -> Result<Self, PrfProviderError> {
        let secret_path = data_dir.join(SECRET_FILE_NAME);

        let secret = if secret_path.exists() {
            // Read existing secret
            let bytes = fs::read(&secret_path).map_err(|e| {
                PrfProviderError::Generic(format!("Failed to read secret file: {e}"))
            })?;

            if bytes.len() != 32 {
                return Err(PrfProviderError::Generic(format!(
                    "Invalid secret file: expected 32 bytes, got {}",
                    bytes.len()
                )));
            }

            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            secret
        } else {
            // Generate new random secret
            let mut secret = [0u8; 32];
            thread_rng().fill_bytes(&mut secret);

            // Ensure data directory exists
            fs::create_dir_all(data_dir).map_err(|e| {
                PrfProviderError::Generic(format!("Failed to create data directory: {e}"))
            })?;

            // Save secret to file
            fs::write(&secret_path, secret).map_err(|e| {
                PrfProviderError::Generic(format!("Failed to write secret file: {e}"))
            })?;

            secret
        };

        Ok(Self { secret })
    }
}

impl FilePrfProvider {
    fn derive_one(&self, salt: &str) -> Vec<u8> {
        let mut engine = HmacEngine::<sha256::Hash>::new(&self.secret);
        engine.input(salt.as_bytes());
        let hmac: Hmac<sha256::Hash> = Hmac::from_engine(engine);
        hmac.to_byte_array().to_vec()
    }
}

#[async_trait::async_trait]
impl PrfProvider for FilePrfProvider {
    async fn derive_seeds(
        &self,
        request: DeriveSeedsRequest,
    ) -> Result<DeriveSeedsOutput, PrfProviderError> {
        // File-backed derivation has no concept of an OS picker; the
        // per-call allow-list and immediate-mediation hint are no-ops
        // here. No credential ID exists for a file-backed secret.
        Ok(DeriveSeedsOutput {
            seeds: request.salts.iter().map(|s| self.derive_one(s)).collect(),
            credential_id: None,
        })
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        Ok(true)
    }
}
