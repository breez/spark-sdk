use crate::Seed;

/// A wallet derived from a passkey.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Wallet {
    pub seed: Seed,
    /// Label used for derivation: user-provided or the default.
    pub label: String,
}

/// Request for [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SetupWalletRequest {
    /// Wallet label. Unset uses the configured default label.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,
    /// Publish the label to Nostr after deriving. Leave false for
    /// speculative derivations (cold restore).
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub publish_label: bool,

    /// Restrict the assertion to these credential IDs. Useful for
    /// server-driven flows that resolve the credential set out-of-band.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credentials: Vec<Vec<u8>>,

    /// Prefer credentials already on this device over the cross-device
    /// picker. Unset uses the platform default.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Response from [`crate::passkey::Passkey::setup_wallet`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WalletSetup {
    pub wallet: Wallet,
    /// Credential that derived this wallet. Absent when the provider
    /// does not surface it.
    pub credential_id: Option<Vec<u8>>,
}

/// Derived seeds plus the credential observed in the same assertion.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeriveSeedsOutput {
    pub seeds: Vec<Vec<u8>>,
    /// Absent when the provider does not surface it.
    pub credential_id: Option<Vec<u8>>,
}

/// A passkey credential from a register or sign-in ceremony.
/// `credential_id` is always set; the attestation fields are
/// populated on registration and absent on sign-in (an assertion
/// carries no attestation). Persist `credential_id` to drive
/// `exclude_credentials` / `allow_credentials` on later calls.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyCredential {
    /// The credential used on sign-in or created on registration.
    pub credential_id: Vec<u8>,
    /// `WebAuthn` user handle, provider-minted at registration.
    /// Absent on sign-in.
    pub user_id: Option<Vec<u8>>,
    /// Authenticator AAGUID. A display hint only: the attestation is
    /// unverified. Absent on sign-in.
    pub aaguid: Option<Vec<u8>>,
    /// Whether the credential is eligible for cloud backup / sync.
    /// Absent on sign-in.
    pub backup_eligible: Option<bool>,
}

impl PasskeyCredential {
    /// Build from a bare credential ID observed during a sign-in
    /// assertion, where no attestation is available.
    pub(crate) fn from_credential_id(credential_id: Vec<u8>) -> Self {
        Self {
            credential_id,
            user_id: None,
            aaguid: None,
            backup_eligible: None,
        }
    }
}

/// Relying Party and user identity for the built-in passkey provider.
/// Applies only when a binding builds the provider for you (the
/// zero-config path); a provider you construct yourself owns these and
/// ignores them.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyProviderOptions {
    /// Relying Party ID. Unset uses the Breez shared RP
    /// (`keys.breez.technology`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub rp_id: Option<String>,

    /// Relying Party name. Unset uses `"Breez"`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub rp_name: Option<String>,

    /// `WebAuthn` `user.name`: the account identifier the OS sign-in
    /// picker shows beneath the display name, typically an email or
    /// handle (e.g. `john@doe.com`). Set a stable per-user
    /// value to keep each registration a distinct entry. Unset uses
    /// `rp_name`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_name: Option<String>,

    /// `WebAuthn` `user.display_name`: the human-friendly name the
    /// picker shows most prominently (e.g. `John Doe`). Unset uses
    /// `user_name`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_display_name: Option<String>,
}

/// Configuration for the passkey client.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyConfig {
    /// Default wallet label when a call provides none. Unset uses
    /// `"Default"`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_label: Option<String>,

    /// Relying Party and user identity for the built-in provider, used
    /// on the zero-config path. Ignored when you inject your own
    /// provider.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub provider_options: Option<PasskeyProviderOptions>,
}
