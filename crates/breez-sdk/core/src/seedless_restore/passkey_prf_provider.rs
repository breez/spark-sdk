use super::error::PasskeyPrfError;

/// Trait for passkey PRF (Pseudo-Random Function) operations.
///
/// Platforms must implement this trait to provide passkey PRF functionality.
/// The implementation is responsible for:
/// - Authenticating the user via platform-specific passkey APIs (`WebAuthn`, native passkey managers)
/// - Evaluating the PRF extension with the provided salt
/// - Returning the 32-byte PRF output
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait PasskeyPrfProvider: Send + Sync {
    /// Derive a 32-byte seed from passkey PRF with the given salt.
    ///
    /// The platform authenticates the user via passkey and evaluates the PRF extension.
    /// The salt is used as input to the PRF to derive a deterministic output.
    ///
    /// # Arguments
    /// * `salt` - The salt string to use for PRF evaluation
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - The 32-byte PRF output
    /// * `Err(PasskeyPrfError)` - If authentication fails or PRF is not supported
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError>;

    /// Check if a PRF-capable passkey is available on this device.
    ///
    /// This allows applications to gracefully degrade if passkey PRF is not supported.
    ///
    /// # Returns
    /// * `Ok(true)` - PRF-capable passkey is available
    /// * `Ok(false)` - No PRF-capable passkey available
    /// * `Err(PasskeyPrfError)` - If the check fails
    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError>;
}
