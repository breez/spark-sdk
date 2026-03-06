use thiserror::Error;

/// Error type for passkey PRF operations.
/// Platforms implement `PasskeyPrfProvider` and return this error type.
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PasskeyPrfError {
    /// PRF extension is not supported by the authenticator
    #[error("PRF not supported by authenticator")]
    PrfNotSupported,

    /// User cancelled the authentication
    #[error("User cancelled authentication")]
    UserCancelled,

    /// No credential found
    #[error("Credential not found")]
    CredentialNotFound,

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// PRF evaluation failed
    #[error("PRF evaluation failed: {0}")]
    PrfEvaluationFailed(String),

    /// Generic error
    #[error("Passkey error: {0}")]
    Generic(String),
}

/// Error type for passkey operations.
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PasskeyError {
    /// Passkey PRF provider error
    #[error("PRF error: {0}")]
    PrfError(#[from] PasskeyPrfError),

    /// Nostr relay connection failed
    #[error("Nostr relay connection failed: {0}")]
    RelayConnectionFailed(String),

    /// Failed to publish to Nostr
    #[error("Nostr write failed: {0}")]
    NostrWriteFailed(String),

    /// Failed to query from Nostr
    #[error("Nostr read failed: {0}")]
    NostrReadFailed(String),

    /// Key derivation error
    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),

    /// Invalid PRF output (wrong size, etc.)
    #[error("Invalid PRF output: {0}")]
    InvalidPrfOutput(String),

    /// BIP39 mnemonic generation error
    #[error("Mnemonic error: {0}")]
    MnemonicError(String),

    /// Invalid salt input
    #[error("Invalid salt: {0}")]
    InvalidSalt(String),

    /// Generic error
    #[error("Passkey error: {0}")]
    Generic(String),
}

impl From<bip39::Error> for PasskeyError {
    fn from(e: bip39::Error) -> Self {
        PasskeyError::MnemonicError(e.to_string())
    }
}

impl From<bitcoin::bip32::Error> for PasskeyError {
    fn from(e: bitcoin::bip32::Error) -> Self {
        PasskeyError::KeyDerivationError(e.to_string())
    }
}
