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
/// seeds plus the credential ID observed in the same assertion. The
/// credential ID is absent when the provider does not surface it
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

/// Configuration for the passkey client. Carries the label-store
/// default plus the Relying Party identity used by the binding-level
/// zero-config `PasskeyClient` constructors.
///
/// `rp_id` / `rp_name` are consumed only when a binding builds the
/// built-in provider for you (the zero-config constructor / builder
/// without an injected provider). When you construct a `PasskeyProvider`
/// yourself, that provider owns its RP and these fields are ignored; the
/// core [`crate::passkey::PasskeyClient`] likewise ignores them because
/// it receives a ready-made provider and only reads `default_label`.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyConfig {
    /// Wallet label used when `register` / `sign_in` receive no label.
    /// Unset uses the internal `DEFAULT_LABEL` (`"Default"`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_label: Option<String>,

    /// Relying Party ID for the built-in provider on the zero-config
    /// path. Unset falls back to the Breez shared RP
    /// (`keys.breez.technology`). Ignored when a provider is supplied
    /// directly.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub rp_id: Option<String>,

    /// Relying Party name for the built-in provider on the zero-config
    /// path. Unset falls back to the SDK default (`"Breez"`). Ignored
    /// when a provider is supplied directly.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub rp_name: Option<String>,
}
