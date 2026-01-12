//! File-based implementation of `PasskeyPrfProvider`.
//!
//! This module provides a software-based implementation of the `PasskeyPrfProvider` trait
//! using a secret file stored in the data directory. This enables seedless wallet
//! restore without requiring hardware security keys.
//!
//! The secret is stored in a file called `seedless-restore-secret` in the data directory.
//! If the file doesn't exist, a random 32-byte secret is generated.

use std::fs;
use std::path::PathBuf;

use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use breez_sdk_spark::seedless_restore::{PasskeyPrfError, PasskeyPrfProvider};
use rand::{RngCore, thread_rng};

/// File name for the seed restore secret.
const SECRET_FILE_NAME: &str = "seedless-restore-secret";

/// File-based implementation of `PasskeyPrfProvider`.
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
    /// Returns `PasskeyPrfError::Generic` if file operations fail.
    pub fn new(data_dir: &PathBuf) -> Result<Self, PasskeyPrfError> {
        let secret_path = data_dir.join(SECRET_FILE_NAME);

        let secret = if secret_path.exists() {
            // Read existing secret
            let bytes = fs::read(&secret_path).map_err(|e| {
                PasskeyPrfError::Generic(format!("Failed to read secret file: {e}"))
            })?;

            if bytes.len() != 32 {
                return Err(PasskeyPrfError::Generic(format!(
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
                PasskeyPrfError::Generic(format!("Failed to create data directory: {e}"))
            })?;

            // Save secret to file
            fs::write(&secret_path, secret).map_err(|e| {
                PasskeyPrfError::Generic(format!("Failed to write secret file: {e}"))
            })?;

            secret
        };

        Ok(Self { secret })
    }
}

#[async_trait::async_trait]
impl PasskeyPrfProvider for FilePrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        // Use HMAC-SHA256(secret, salt) as the PRF output
        let mut engine = HmacEngine::<sha256::Hash>::new(&self.secret);
        engine.input(salt.as_bytes());
        let hmac: Hmac<sha256::Hash> = Hmac::from_engine(engine);

        Ok(hmac.to_byte_array().to_vec())
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        // File-based PRF is always available once initialized
        Ok(true)
    }
}
