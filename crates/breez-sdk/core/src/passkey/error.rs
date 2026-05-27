use thiserror::Error;

/// Coarse classification of a passkey error by the UX reaction it
/// warrants. The variant names the action to take, not the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ErrorKind {
    /// User dismissed the prompt. Do not auto-retry.
    Cancel,
    /// No matching credential on this device. Offer to register one.
    NoCredential,
    /// Authenticator lacks the PRF extension. Fall back to a non-passkey
    /// flow or guide the user to another credential provider.
    PrfUnsupported,
    /// PRF is supported but evaluation failed. Often transient: retrying
    /// the ceremony may succeed.
    PrfFailed,
    /// Platform / app setup is wrong (entitlement, assetlinks, rpId
    /// scope). Not retryable until the integrator fixes it.
    Configuration,
    /// An existing credential matched. Route the user to sign-in.
    AlreadyExists,
    /// The prompt closed on the platform inactivity timeout with no user
    /// action. Unlike `Cancel`, safe to auto-retry or re-prompt.
    Timeout,
    /// The ceremony failed for a security or state reason. Offer a retry;
    /// if it persists, the credential or RP setup may be at fault.
    AuthFailure,
    /// Failure the caller can't act on. Show a generic "try again".
    Internal,
}

/// Failures from a passkey PRF operation. Each platform normalizes its
/// native errors into these variants so callers match one taxonomy
/// everywhere; anything unclassifiable becomes [`Generic`](Self::Generic).
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PrfProviderError {
    #[error("PRF not supported by authenticator")]
    PrfNotSupported,

    #[error("User cancelled authentication")]
    UserCancelled,

    /// The prompt closed on the platform inactivity timeout, with no
    /// user action. Unlike `UserCancelled`, safe to auto-retry.
    #[error("Authenticator timed out")]
    UserTimedOut,

    #[error("Credential not found: {0}")]
    CredentialNotFound(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("PRF evaluation failed: {0}")]
    PrfEvaluationFailed(String),

    /// Platform / app setup is wrong: missing AASA entitlement, invalid
    /// assetlinks.json, or misconfigured RP ID.
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// An existing credential matched. Route the user to sign-in.
    #[error("Credential already exists: {0}")]
    CredentialAlreadyExists(String),

    #[error("Passkey error: {0}")]
    Generic(String),
}

impl PrfProviderError {
    /// Coarse classification so callers branch on a small, actionable
    /// enum instead of every variant.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UserCancelled => ErrorKind::Cancel,
            Self::UserTimedOut => ErrorKind::Timeout,
            Self::CredentialNotFound(_) => ErrorKind::NoCredential,
            Self::PrfNotSupported => ErrorKind::PrfUnsupported,
            Self::PrfEvaluationFailed(_) => ErrorKind::PrfFailed,
            Self::Configuration(_) => ErrorKind::Configuration,
            Self::CredentialAlreadyExists(_) => ErrorKind::AlreadyExists,
            Self::AuthenticationFailed(_) => ErrorKind::AuthFailure,
            Self::Generic(_) => ErrorKind::Internal,
        }
    }
}

/// Error type for passkey operations.
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PasskeyError {
    /// Error raised by the underlying [`crate::passkey::PrfProvider`].
    #[error("PRF error: {0}")]
    Prf(#[from] PrfProviderError),

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

impl PasskeyError {
    /// Coarse classification of the underlying failure. Non-PRF
    /// variants (Nostr, key derivation, mnemonic) all map to
    /// `Internal` because they're caused by SDK / network state, not
    /// authenticator state: the caller should surface a generic
    /// retry / "try again later" UI.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Prf(inner) => inner.kind(),
            _ => ErrorKind::Internal,
        }
    }
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
