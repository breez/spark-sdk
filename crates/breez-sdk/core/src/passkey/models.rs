use crate::Seed;

/// A wallet derived from a passkey.
///
/// Contains the derived seed and the label used during derivation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Wallet {
    /// The derived seed.
    pub seed: Seed,
    /// The label used for derivation (either user-provided or the default).
    pub label: String,
}

/// Request for [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SetupWalletRequest {
    /// User-chosen label. Defaults to the configured default label
    /// (`PasskeyConfig::default_label`, or `"Default"` when unset) when
    /// `None`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,
    /// Whether to publish `label` to Nostr after deriving. Pass `false`
    /// for speculative derivations (cold restore).
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub publish_label: bool,

    /// Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::allow_credentials`].
    /// Useful for server-driven flows that resolve the credential set
    /// out-of-band.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credentials: Vec<Vec<u8>>,

    /// Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::prefer_immediately_available_credentials`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Response from [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WalletSetup {
    pub wallet: Wallet,
    /// Credential ID observed during the assertion that derived this
    /// wallet, when the provider surfaces it. Feeds
    /// [`SignInResponse::credential_id`](crate::passkey::SignInResponse).
    pub credential_id: Option<Vec<u8>>,
}

/// Output of [`crate::passkey::PrfProvider::derive_seeds`]: the derived
/// seeds plus the credential ID observed in the same assertion.
/// `credential_id` is `None` when the provider does not surface it
/// (CLI / hardware backends).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeriveSeedsOutput {
    pub seeds: Vec<Vec<u8>>,
    pub credential_id: Option<Vec<u8>>,
}

/// Result of a successful [`crate::passkey::PrfProvider::create_passkey`].
///
/// `user_id` is the `WebAuthn` `user.id` (user handle) the provider
/// generated for this credential. Always returned so hosts can
/// correlate server-side; never host-supplied (see the SDK's
/// passkey guide for the rationale).
///
/// `aaguid` (16-byte authenticator identifier) and `backup_eligible`
/// (BE flag) are parsed from authenticator data when available; they
/// are `None` on platforms that don't expose attestation data (e.g.
/// Safari without `getAuthenticatorData()`). Use only as display
/// hints, never for trust decisions: AAGUID is unverified attestation.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisteredCredential {
    pub credential_id: Vec<u8>,
    /// The `WebAuthn` `user.id` (user handle) the provider generated
    /// for this credential. 1-64 bytes per spec.
    pub user_id: Vec<u8>,
    pub aaguid: Option<Vec<u8>>,
    pub backup_eligible: Option<bool>,
}

/// Optional configuration for [`crate::passkey::PasskeyClient::new`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyConfig {
    /// Wallet label used when `register` / `sign_in` receive
    /// `label = None`. `None` ⇒ internal `DEFAULT_LABEL` (`"Default"`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_label: Option<String>,
}
