use thiserror::Error;

/// Coarse classification of a passkey error: what the caller should
/// do next. One value per distinct user/UX reaction. Map to your own
/// presentation; the variant name carries the action, not the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ErrorKind {
    /// The user dismissed the authenticator prompt. Do not auto-retry.
    Cancel,
    /// No matching credential on this device. Offer to register a new one.
    NoCredential,
    /// The authenticator does not implement the PRF extension. Fall back
    /// to a non-passkey flow or guide the user to switch credential
    /// providers (e.g. iCloud Keychain on iOS).
    PrfUnsupported,
    /// Platform / app configuration is wrong (entitlement, assetlinks,
    /// rpId scope). Not retryable until the integrator fixes setup.
    Configuration,
    /// `excludeCredentialIds` matched an existing credential. Route the
    /// user to the sign-in path.
    AlreadyExists,
    /// The OS biometric prompt timed out (the user did not interact
    /// within the platform's inactivity window, typically ~55 seconds).
    /// Distinct from `Cancel`: the user did not actively dismiss the
    /// prompt. Hosts may auto-retry or surface a re-prompt UI without
    /// treating it as user intent to abandon.
    Timeout,
    /// Platform or library failure the caller can't act on. Surface a
    /// generic "try again" UI; diagnostic detail is in the variant
    /// payload for logs.
    Internal,
}

/// Error type for passkey PRF operations.
/// Platforms implement `PrfProvider` and return this error type.
#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum PrfProviderError {
    /// PRF extension is not supported by the authenticator
    #[error("PRF not supported by authenticator")]
    PrfNotSupported,

    /// User cancelled the authentication
    #[error("User cancelled authentication")]
    UserCancelled,

    /// The OS biometric prompt timed out without user interaction.
    /// On iOS / Android this is the platform's biometric inactivity
    /// timeout (typically ~55 seconds): the prompt was up but the
    /// user neither approved nor dismissed it. Distinct from
    /// `UserCancelled`, which means the user actively dismissed the
    /// prompt. Hosts can use this signal to auto-retry or surface a
    /// re-prompt UI without treating it as user intent to abandon.
    #[error("Authenticator timed out")]
    UserTimedOut,

    /// No credential found
    #[error("Credential not found: {0}")]
    CredentialNotFound(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// PRF evaluation failed
    #[error("PRF evaluation failed: {0}")]
    PrfEvaluationFailed(String),

    /// Platform or app configuration error (e.g. missing AASA entitlement,
    /// invalid assetlinks.json, misconfigured RP ID).
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// An entry in `excludeCredentialIds` matched a credential
    /// already on the device. Route the user to sign-in.
    #[error("Credential already exists: {0}")]
    CredentialAlreadyExists(String),

    /// Generic error
    #[error("Passkey error: {0}")]
    Generic(String),
}

impl PrfProviderError {
    /// Coarse classification for the caller. Lets hosts branch on a
    /// small, actionable enum instead of pattern-matching every
    /// variant.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UserCancelled => ErrorKind::Cancel,
            Self::UserTimedOut => ErrorKind::Timeout,
            Self::CredentialNotFound(_) => ErrorKind::NoCredential,
            Self::PrfNotSupported | Self::PrfEvaluationFailed(_) => ErrorKind::PrfUnsupported,
            Self::Configuration(_) => ErrorKind::Configuration,
            Self::CredentialAlreadyExists(_) => ErrorKind::AlreadyExists,
            Self::AuthenticationFailed(_) | Self::Generic(_) => ErrorKind::Internal,
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
    /// authenticator state — the caller should surface a generic
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
