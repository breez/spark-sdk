//! Passkey-based wallet operations using `WebAuthn` PRF extension.
//!
//! This module implements the [seedless-restore spec](https://github.com/breez/seedless-restore)
//! for deriving wallet seeds from passkey PRF outputs and storing/discovering
//! salts via Nostr relays.
//!
//! # Overview
//!
//! The passkey flow works as follows:
//!
//! 1. **Account Master Derivation**: `PRF(passkey, magic_salt)` produces a 32-byte
//!    account master used to derive the Nostr identity at `m/44'/1237'/55'/0/0`.
//!
//! 2. **Salt Storage**: User-provided salts are published as Nostr kind-1 events,
//!    allowing discovery during wallet restore.
//!
//! 3. **Wallet Seed Derivation**: `PRF(passkey, user_salt)` produces 32 bytes, the first
//!    16 of which are converted to a 12-word BIP39 mnemonic.
//!
//! # Platform Implementation
//!
//! Platforms must implement the [`PrfProvider`] trait to derive wallet seeds.
//! The built-in `PasskeyProvider` on each platform satisfies this contract
//! by authenticating with a platform passkey; custom CLI providers (`YubiKey`,
//! FIDO2, file-backed) implement the same trait for deterministic derivation
//! from other sources. The SDK orchestrates the flow, while implementations
//! handle the actual derivation.
//!
//! # Example
//!
//! ```ignore
//! use breez_sdk_spark::passkey::{PasskeyClient, SignInRequest};
//!
//! // Platform provides a PrfProvider implementation
//! let prf_provider = Arc::new(MyPrfProvider::new());
//!
//! let client = PasskeyClient::new(prf_provider, None, None);
//!
//! // Sign in for a known label (fast path; no label-store query)
//! let response = client
//!     .sign_in(SignInRequest {
//!         label: Some("my-wallet".to_string()),
//!         ..Default::default()
//!     })
//!     .await?;
//! ```

mod derivation;
mod error;
mod models;
mod nostr_client;
mod passkey_client;
mod passkey_prf_provider;

pub use derivation::ACCOUNT_MASTER_SALT;
use derivation::prf_to_mnemonic;
pub use error::{ErrorKind, PasskeyError, PrfProviderError};
pub use models::{
    CreatePasskeyOutput, DeriveSeedsOutput, PasskeyConfig, PasskeyCredential,
    PasskeyProviderOptions, SetupWalletRequest, Wallet, WalletSetup,
};
pub use passkey_client::{
    ConnectWithPasskeyRequest, ConnectWithPasskeyResponse, PasskeyAvailability, PasskeyClient,
    PasskeyLabels, RegisterRequest, RegisterResponse, SignInRequest, SignInResponse,
};
pub use passkey_prf_provider::{DeriveSeedsRequest, DomainAssociation, PrfProvider};

use std::sync::Arc;

use platform_utils::tokio;
use tokio::sync::RwLock;
use tracing::warn;

use crate::Seed;
use derivation::derive_nostr_keypair;
use nostr_client::{LabelStore, NostrSaltClient};

/// Builds the per-identity [`LabelStore`] from the Nostr keys derived in a
/// PRF ceremony plus the optional Breez API key. The default builds a
/// network-backed [`NostrSaltClient`]; tests inject an in-memory double so
/// unit tests never reach the relays.
type LabelStoreBuilder =
    Arc<dyn Fn(nostr::Keys, Option<String>) -> Arc<dyn LabelStore> + Send + Sync>;

/// Default store builder: a network-backed [`NostrSaltClient`] reaching the
/// relays through `proxy`, when one is configured.
fn default_label_store_builder(proxy: Option<crate::ProxyConfig>) -> LabelStoreBuilder {
    Arc::new(move |keys, breez_api_key| {
        Arc::new(NostrSaltClient::new(keys, breez_api_key, proxy.clone())) as Arc<dyn LabelStore>
    })
}

/// The default label used when none is provided.
pub(super) const DEFAULT_LABEL: &str = "Default";

/// Maximum allowed label length in bytes.
const MAX_LABEL_LENGTH: usize = 1024;

/// Validate a user-provided label string.
fn validate_label(label: &str) -> Result<(), PasskeyError> {
    if label.is_empty() {
        return Err(PasskeyError::InvalidSalt(
            "label must not be empty".to_string(),
        ));
    }
    if label.len() > MAX_LABEL_LENGTH {
        return Err(PasskeyError::InvalidSalt(format!(
            "label exceeds maximum length of {MAX_LABEL_LENGTH} bytes"
        )));
    }
    Ok(())
}

/// Orchestrates passkey-based wallet derivation and label management.
/// Composes a [`PrfProvider`] (for the deterministic 32-byte
/// derivation) with the internal Nostr-backed label store.
///
/// The passkey-derived Nostr keys (derived from the account-master
/// PRF salt) are cached after the first derivation so subsequent
/// label ops on the same instance need no additional PRF prompts.
#[derive(Clone)]
pub(crate) struct Passkey {
    prf_provider: Arc<dyn PrfProvider>,
    breez_api_key: Option<String>,
    default_label: String,
    /// Cached label store, lazily built on first label op and
    /// re-bindable by [`Self::setup_wallet`] when a new derivation
    /// resolves to a different identity (e.g. a follow-up register
    /// on the same [`PasskeyClient`] instance).
    nostr_client: Arc<RwLock<Option<Arc<dyn LabelStore>>>>,
    /// Builds the label store once the identity keys are derived.
    /// Swapped in tests for an in-memory double.
    store_builder: LabelStoreBuilder,
}

impl Passkey {
    /// Create a new `Passkey`.
    ///
    /// `breez_api_key` enables authenticated (NIP-42) access to the
    /// Breez relay for label storage. Pass `None` to use public relays
    /// only.
    pub fn new(
        prf_provider: Arc<dyn PrfProvider>,
        breez_api_key: Option<String>,
        config: Option<PasskeyConfig>,
    ) -> Result<Self, PasskeyError> {
        let config = config.unwrap_or_default();
        config.validate()?;
        Ok(Self {
            prf_provider,
            breez_api_key,
            default_label: config
                .default_label
                .filter(|s| validate_label(s).is_ok())
                .unwrap_or_else(|| DEFAULT_LABEL.to_string()),
            nostr_client: Arc::new(RwLock::new(None)),
            store_builder: default_label_store_builder(config.proxy),
        })
    }

    /// Construct with a custom [`LabelStore`] builder. Tests inject an
    /// in-memory store so unit tests never reach the Nostr relays.
    #[cfg(test)]
    pub fn new_with_store_builder(
        prf_provider: Arc<dyn PrfProvider>,
        breez_api_key: Option<String>,
        config: Option<PasskeyConfig>,
        store_builder: LabelStoreBuilder,
    ) -> Result<Self, PasskeyError> {
        Ok(Self {
            store_builder,
            ..Self::new(prf_provider, breez_api_key, config)?
        })
    }

    /// Returns a reference to the underlying [`PrfProvider`].
    pub fn prf_provider(&self) -> &Arc<dyn PrfProvider> {
        &self.prf_provider
    }

    /// Derive (or retrieve cached) Nostr keys for the passkey's
    /// label-store identity. Test-only helper that exercises the same
    /// cache the runtime path goes through; production code reads via
    /// [`Self::nostr_client`].
    #[cfg(test)]
    async fn derive_keys(&self) -> Result<nostr::Keys, PasskeyError> {
        let client = self.nostr_client().await?;
        Ok(client.signing_keys())
    }

    /// Lazily build (or retrieve cached) the label-store client.
    async fn nostr_client(&self) -> Result<Arc<dyn LabelStore>, PasskeyError> {
        // Fast path: read-lock check.
        if let Some(c) = self.nostr_client.read().await.as_ref() {
            return Ok(c.clone());
        }
        // Slow path: take the write lock and re-check (another task may
        // have initialized in the meantime).
        let mut w = self.nostr_client.write().await;
        if let Some(c) = w.as_ref() {
            return Ok(c.clone());
        }
        let mut output = self
            .prf_provider
            .derive_seeds(DeriveSeedsRequest {
                salts: vec![ACCOUNT_MASTER_SALT.to_string()],
                ..Default::default()
            })
            .await?;
        let account_master = output.seeds.pop().ok_or_else(|| {
            PrfProviderError::Generic("derive_seeds returned no output".to_string())
        })?;
        let keys = derive_nostr_keypair(&account_master)?;
        let client = (self.store_builder)(keys, self.breez_api_key.clone());
        *w = Some(client.clone());
        Ok(client)
    }

    /// Resolve a caller-supplied label against the configured default and
    /// validate it. Exposed so a caller can reject a bad label before
    /// driving any ceremony.
    pub(crate) fn resolve_label(&self, label: Option<String>) -> Result<String, PasskeyError> {
        let label = label.unwrap_or_else(|| self.default_label.clone());
        validate_label(&label)?;
        Ok(label)
    }

    /// Derive the Nostr identity and the wallet seed for `request.label`
    /// in one PRF ceremony (dual-salt where the platform supports it).
    /// Primes the identity cache so subsequent [`Self::list_labels`] /
    /// [`Self::store_label`] need no extra PRF prompts.
    /// `publish_label = false` skips the Nostr write: used by
    /// speculative cold-restore.
    pub async fn setup_wallet(
        &self,
        request: SetupWalletRequest,
    ) -> Result<WalletSetup, PasskeyError> {
        let label = self.resolve_label(request.label)?;
        let salts = Self::wallet_salts(&label);
        let expected = salts.len();

        let DeriveSeedsOutput {
            seeds,
            credential_id,
        } = self
            .prf_provider
            .derive_seeds(DeriveSeedsRequest {
                salts,
                allow_credentials: request.allow_credentials,
                prefer_immediately_available_credentials: request
                    .prefer_immediately_available_credentials,
            })
            .await?;
        if seeds.len() != expected {
            return Err(PasskeyError::Prf(PrfProviderError::Generic(format!(
                "derive_seeds returned {} outputs, expected {expected}",
                seeds.len()
            ))));
        }

        self.finish_wallet_setup(label, seeds, credential_id, request.publish_label)
            .await
    }

    /// Salts backing one wallet, in the order `finish_wallet_setup`
    /// consumes them: `[account_master, label]`. A dual-salt ceremony
    /// covers both at once where the platform supports it.
    pub(crate) fn wallet_salts(label: &str) -> Vec<String> {
        vec![ACCOUNT_MASTER_SALT.to_string(), label.to_string()]
    }

    /// Everything after the PRF outputs exist, so a caller that already
    /// has them (a create ceremony that evaluated PRF inline) reaches the
    /// same wallet without a second ceremony.
    pub(crate) async fn finish_wallet_setup(
        &self,
        label: String,
        seeds: Vec<Vec<u8>>,
        credential_id: Option<Vec<u8>>,
        publish_label: bool,
    ) -> Result<WalletSetup, PasskeyError> {
        // Prime (or rebind) the Nostr client cache with the keys derived
        // in this ceremony so subsequent label ops don't trigger another
        // PRF, even when this `PasskeyClient` is reused across credentials
        // (a follow-up `register` resolves to a different identity).
        let keys = derive_nostr_keypair(&seeds[0])?;
        *self.nostr_client.write().await =
            Some((self.store_builder)(keys, self.breez_api_key.clone()));

        // Build the wallet before reaching out to the label store so a
        // transient publish failure can't burn the PRF ceremony.
        let mnemonic = prf_to_mnemonic(&seeds[1])?;
        let wallet = Wallet {
            seed: Seed::Mnemonic {
                mnemonic,
                passphrase: None,
            },
            label: label.clone(),
        };

        if publish_label && let Ok(client) = self.nostr_client().await {
            // The label publish is a non-fatal discovery convenience, so run
            // it off the critical path rather than blocking on slow relays.
            // store_label is idempotent, so the bounded retry is safe.
            let label = label.clone();
            tokio::spawn(async move {
                for attempt in 0..3u32 {
                    match client.store_label(&label).await {
                        Ok(()) => return,
                        Err(e) => warn!(
                            "setup_wallet: background store_label attempt {attempt} failed: {e}"
                        ),
                    }
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                    }
                }
            });
        }

        Ok(WalletSetup {
            wallet,
            credential_id,
        })
    }

    /// List labels published for this passkey's identity. Requires
    /// one PRF call to derive the identity (cached after the first
    /// call on this `Passkey` instance).
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        let client = self.nostr_client().await?;
        client.list_labels().await
    }

    /// Idempotently publish `label` for this passkey's identity.
    /// Requires one PRF call to derive the identity (cached).
    pub async fn store_label(&self, label: String) -> Result<(), PasskeyError> {
        validate_label(&label)?;
        let client = self.nostr_client().await?;
        client.store_label(&label).await
    }

    /// Map [`PrfProvider`] capability + domain-association probes into a
    /// single [`PasskeyAvailability`].
    pub async fn check_availability(&self) -> Result<PasskeyAvailability, PasskeyError> {
        if !self.prf_provider.is_supported().await? {
            return Ok(PasskeyAvailability::PrfUnsupported);
        }
        let association = self.prf_provider.check_domain_association().await?;
        Ok(match association {
            DomainAssociation::Associated => PasskeyAvailability::Available,
            DomainAssociation::NotAssociated { source, reason } => {
                PasskeyAvailability::NotAssociated { source, reason }
            }
            DomainAssociation::Skipped { reason } => PasskeyAvailability::Skipped { reason },
        })
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    // Mock implementation of PrfProvider for testing
    struct MockPrfProvider {
        seed: [u8; 32],
    }

    impl MockPrfProvider {
        fn new(seed: [u8; 32]) -> Self {
            Self { seed }
        }
    }

    #[macros::async_trait]
    impl PrfProvider for MockPrfProvider {
        async fn derive_seeds(
            &self,
            request: DeriveSeedsRequest,
        ) -> Result<DeriveSeedsOutput, PrfProviderError> {
            Ok(DeriveSeedsOutput {
                seeds: request
                    .salts
                    .into_iter()
                    .map(|_| self.seed.to_vec())
                    .collect(),
                credential_id: None,
            })
        }

        async fn is_supported(&self) -> Result<bool, PrfProviderError> {
            Ok(true)
        }
    }

    // Mock that always fails - for testing error propagation
    struct FailingPrfProvider {
        error: PrfProviderError,
    }

    impl FailingPrfProvider {
        fn new(error: PrfProviderError) -> Self {
            Self { error }
        }
    }

    #[macros::async_trait]
    impl PrfProvider for FailingPrfProvider {
        async fn derive_seeds(
            &self,
            _request: DeriveSeedsRequest,
        ) -> Result<DeriveSeedsOutput, PrfProviderError> {
            Err(self.error.clone())
        }

        async fn is_supported(&self) -> Result<bool, PrfProviderError> {
            Err(self.error.clone())
        }
    }

    struct UnavailablePrfProvider;

    #[macros::async_trait]
    impl PrfProvider for UnavailablePrfProvider {
        async fn derive_seeds(
            &self,
            _request: DeriveSeedsRequest,
        ) -> Result<DeriveSeedsOutput, PrfProviderError> {
            Err(PrfProviderError::PrfNotSupported)
        }

        async fn is_supported(&self) -> Result<bool, PrfProviderError> {
            Ok(false)
        }
    }

    #[macros::test_all]
    fn test_passkey_new() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let config = PasskeyConfig::default();

        let _passkey = Passkey::new(prf_provider, None, Some(config)).unwrap();
    }

    fn passkey_proxy(username: Option<&str>, password: Option<&str>) -> crate::ProxyConfig {
        crate::ProxyConfig {
            host: "127.0.0.1".to_string(),
            port: 9050,
            username: username.map(ToString::to_string),
            password: password.map(ToString::to_string),
        }
    }

    /// The label is a PRF salt the wallet seed derives from, and the relay
    /// directory is the only copy of it, so an unpublishable label has to
    /// fail before a wallet exists rather than at the first label op.
    #[macros::test_all]
    fn test_passkey_new_rejects_authenticated_proxy() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let config = PasskeyConfig {
            proxy: Some(passkey_proxy(Some("user"), Some("pass"))),
            ..PasskeyConfig::default()
        };

        match Passkey::new(prf_provider, None, Some(config)).err() {
            Some(PasskeyError::InvalidConfig(m)) => {
                assert!(
                    m.contains("proxy authentication"),
                    "expected a proxy-auth rejection, got: {m}"
                );
            }
            other => panic!("expected an authenticated proxy to be rejected, got {other:?}"),
        }
    }

    /// A permanent misconfiguration, not a transient failure: hosts branch
    /// on `kind()` to decide whether a retry is worth offering.
    #[macros::test_all]
    fn test_invalid_config_is_not_retryable() {
        assert_eq!(
            PasskeyError::InvalidConfig("x".to_string()).kind(),
            crate::passkey::ErrorKind::Configuration
        );
    }

    #[macros::test_all]
    fn test_passkey_new_accepts_unauthenticated_proxy() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let config = PasskeyConfig {
            proxy: Some(passkey_proxy(None, None)),
            ..PasskeyConfig::default()
        };

        // WASM cannot honour any proxy, so the accept case is native-only.
        let result = Passkey::new(prf_provider, None, Some(config));
        if cfg!(all(target_family = "wasm", target_os = "unknown")) {
            assert!(matches!(result, Err(PasskeyError::InvalidConfig(_))));
        } else {
            assert!(result.is_ok());
        }
    }

    #[macros::async_test_all]
    async fn test_check_availability_available() {
        let prf_provider = Arc::new(MockPrfProvider::new([0u8; 32]));
        let passkey = Passkey::new(prf_provider, None, None).unwrap();

        let availability = passkey.check_availability().await.unwrap();
        assert!(matches!(
            availability,
            PasskeyAvailability::Available | PasskeyAvailability::Skipped { .. }
        ));
    }

    #[macros::async_test_all]
    async fn test_check_availability_prf_unsupported() {
        let prf_provider = Arc::new(UnavailablePrfProvider);
        let passkey = Passkey::new(prf_provider, None, None).unwrap();

        let availability = passkey.check_availability().await.unwrap();
        assert!(matches!(availability, PasskeyAvailability::PrfUnsupported));
    }

    #[macros::async_test_all]
    async fn test_check_availability_error() {
        let prf_provider = Arc::new(FailingPrfProvider::new(
            PrfProviderError::AuthenticationFailed("Test error".to_string()),
        ));
        let passkey = Passkey::new(prf_provider, None, None).unwrap();

        let result = passkey.check_availability().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::Prf(PrfProviderError::AuthenticationFailed(_))
        ));
    }

    #[macros::test_all]
    fn test_account_master_salt_is_valid_hex() {
        let decoded = hex::decode(ACCOUNT_MASTER_SALT);
        assert!(decoded.is_ok(), "ACCOUNT_MASTER_SALT should be valid hex");

        let decoded_bytes = decoded.unwrap();
        let decoded_str = String::from_utf8(decoded_bytes);
        assert!(decoded_str.is_ok());
        assert_eq!(decoded_str.unwrap(), "NYOASTRTSAOYN");
    }

    #[macros::async_test_all]
    async fn test_derive_keys_deterministic() {
        let prf_provider1 = Arc::new(MockPrfProvider::new([99u8; 32]));
        let prf_provider2 = Arc::new(MockPrfProvider::new([99u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None, None).unwrap();
        let passkey2 = Passkey::new(prf_provider2, None, None).unwrap();

        let keys1 = passkey1.derive_keys().await.unwrap();
        let keys2 = passkey2.derive_keys().await.unwrap();

        assert_eq!(
            keys1.public_key(),
            keys2.public_key(),
            "Same PRF output should produce same identity"
        );
    }

    #[macros::async_test_all]
    async fn test_derive_keys_different_providers() {
        let prf_provider1 = Arc::new(MockPrfProvider::new([1u8; 32]));
        let prf_provider2 = Arc::new(MockPrfProvider::new([2u8; 32]));

        let passkey1 = Passkey::new(prf_provider1, None, None).unwrap();
        let passkey2 = Passkey::new(prf_provider2, None, None).unwrap();

        let keys1 = passkey1.derive_keys().await.unwrap();
        let keys2 = passkey2.derive_keys().await.unwrap();

        assert_ne!(
            keys1.public_key(),
            keys2.public_key(),
            "Different PRF outputs should produce different identities"
        );
    }
}
