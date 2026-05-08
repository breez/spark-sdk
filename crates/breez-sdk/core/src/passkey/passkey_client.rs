//! High-level passkey orchestration. [`PasskeyClient`] is the ergonomic
//! entry point for hosts: it composes the lower-level [`Passkey`]
//! (label store + identity cache) and the [`PrfProvider`] trait into a
//! handful of named flows that match real onboarding UI states.
//!
//! For the lower-level building blocks, use [`Passkey`] directly.

use std::collections::HashMap;
use std::sync::Arc;

use super::Passkey;
use super::error::PasskeyError;
use super::label_store::LabelStore;
use super::models::{
    CreatePasskeyRequest, NamedSalt, NostrRelayConfig, RegisteredCredential, SetupWalletRequest,
    Wallet,
};
use super::passkey_prf_provider::PrfProvider;

/// Request shape for [`PasskeyClient::register`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterRequest {
    /// User-chosen label for the new wallet. Defaults to
    /// [`DEFAULT_LABEL`] when `None`. Always published to the label
    /// store as part of registration.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,

    /// Extra app-scoped salts to derive in the same PRF ceremony as
    /// the wallet seed. See [`NamedSalt`]; outputs are returned via
    /// [`RegisterResponse::extra_seeds`].
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub extra_salts: Vec<NamedSalt>,

    /// Forwarded to [`PrfProvider::create_passkey`]; routes "this
    /// device already has a credential" to
    /// [`crate::passkey::PrfProviderError::CredentialAlreadyExists`]
    /// so the host can flip to the sign-in path.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub exclude_credential_ids: Vec<Vec<u8>>,

    /// Forwarded to [`CreatePasskeyRequest::user_id`]. Always provide a
    /// fresh random value per call; reusing one across registrations
    /// can silently destroy the prior credential on some authenticators.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_id: Option<Vec<u8>>,

    /// Forwarded to [`CreatePasskeyRequest::user_name`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_name: Option<String>,

    /// Forwarded to [`CreatePasskeyRequest::user_display_name`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub user_display_name: Option<String>,
}

/// Response from [`PasskeyClient::register`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterResponse {
    /// The newly-derived wallet for [`RegisterRequest::label`].
    pub wallet: Wallet,
    /// Metadata for the credential the platform just registered. Hosts
    /// SHOULD persist [`RegisteredCredential::credential_id`] so they
    /// can populate `exclude_credential_ids` on future
    /// [`PasskeyClient::register`] calls.
    pub credential: RegisteredCredential,
    /// 32 bytes per [`NamedSalt`] in [`RegisterRequest::extra_salts`].
    pub extra_seeds: HashMap<String, Vec<u8>>,
}

/// Request shape for [`PasskeyClient::sign_in`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SignInRequest {
    /// If provided, derive directly for this label and skip the
    /// label-store query (fast path; one ceremony, no Nostr round-trip).
    /// Use when the host has the user's label cached locally from a
    /// previous session.
    ///
    /// If `None`, the SDK derives a wallet for [`DEFAULT_LABEL`] **and**
    /// queries the label store for the user's full label set in the
    /// same ceremony. Use when the host has no local state — typically
    /// a fresh install / new device. The host then checks
    /// [`SignInResponse::labels`]: if it contains [`DEFAULT_LABEL`] the
    /// returned wallet is final; otherwise show a picker and call
    /// `sign_in` again with the chosen label.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,

    /// Same as [`RegisterRequest::extra_salts`].
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub extra_salts: Vec<NamedSalt>,
}

/// Response from [`PasskeyClient::sign_in`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SignInResponse {
    /// The derived wallet for the resolved label
    /// ([`SignInRequest::label`] or [`DEFAULT_LABEL`]).
    pub wallet: Wallet,
    /// All labels the user has published. **Empty** when
    /// [`SignInRequest::label`] was provided (fast path; the SDK skips
    /// the label-store query). **Populated** when `label` was `None`,
    /// so the host can show a picker if the default-derived wallet
    /// isn't the one the user wanted. Also empty if the label store
    /// is unreachable on the discovery path.
    pub labels: Vec<String>,
    /// Same as [`RegisterResponse::extra_seeds`].
    pub extra_seeds: HashMap<String, Vec<u8>>,
}

/// High-level orchestration over a [`PrfProvider`] and a
/// [`LabelStore`]. Two named flows match the real onboarding states:
///
/// - [`Self::register`]: first-time setup (create credential + derive
///   wallet + publish label) in one ceremony where the platform
///   supports dual-salt PRF.
/// - [`Self::sign_in`]: returning user. Fast path when the host has
///   the label cached locally; cold-restore-with-discovery when not.
///
/// Construct via [`Self::new`] (default Nostr-backed label store) or
/// [`Self::from_config`] (re-use the SDK's API key).
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PasskeyClient {
    passkey: Passkey,
    prf_provider: Arc<dyn PrfProvider>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl PasskeyClient {
    /// Construct with the default Nostr-backed label store.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(prf_provider: Arc<dyn PrfProvider>, relay_config: Option<NostrRelayConfig>) -> Self {
        let passkey = Passkey::new(Arc::clone(&prf_provider), relay_config);
        Self {
            passkey,
            prf_provider,
        }
    }

    /// First-time setup. Drives [`PrfProvider::create_passkey`] (one
    /// ceremony) followed by the wallet-derivation flow that backs
    /// [`Passkey::setup_wallet`] (one or two ceremonies depending on
    /// `extra_salts` and dual-salt support). The label is always
    /// published on success.
    pub async fn register(
        &self,
        request: RegisterRequest,
    ) -> Result<RegisterResponse, PasskeyError> {
        let credential = self
            .prf_provider
            .create_passkey(CreatePasskeyRequest {
                exclude_credential_ids: request.exclude_credential_ids,
                user_id: request.user_id,
                user_name: request.user_name,
                user_display_name: request.user_display_name,
            })
            .await?;

        let setup = self
            .passkey
            .setup_wallet(SetupWalletRequest {
                label: request.label,
                publish_label: true,
                extra_salts: request.extra_salts,
            })
            .await?;

        Ok(RegisterResponse {
            wallet: setup.wallet,
            credential,
            extra_seeds: setup.extra_seeds,
        })
    }

    /// Returning-user sign-in. Two modes based on whether the host
    /// has the label cached locally:
    ///
    /// - **Fast path** ([`SignInRequest::label`] = `Some(...)`): one
    ///   ceremony, no Nostr round-trip. [`SignInResponse::labels`] is
    ///   empty.
    /// - **Discovery path** ([`SignInRequest::label`] = `None`): one
    ///   ceremony derives a wallet for [`DEFAULT_LABEL`] and the SDK
    ///   queries the label store off the cached identity (no extra
    ///   prompt). [`SignInResponse::labels`] carries the full set; the
    ///   host shows a picker if [`DEFAULT_LABEL`] isn't in it and calls
    ///   `sign_in` again with the chosen label.
    ///
    /// In both modes the label is **not** re-published. Call
    /// [`Self::store_label`] separately if needed.
    ///
    /// The label-store query on the discovery path is best-effort: a
    /// transient failure leaves `labels` empty rather than aborting,
    /// since the speculative wallet is still useful.
    pub async fn sign_in(
        &self,
        request: SignInRequest,
    ) -> Result<SignInResponse, PasskeyError> {
        let discovery = request.label.is_none();

        let setup = self
            .passkey
            .setup_wallet(SetupWalletRequest {
                label: request.label,
                publish_label: false,
                extra_salts: request.extra_salts,
            })
            .await?;

        let labels = if discovery {
            self.passkey.list_labels().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(SignInResponse {
            wallet: setup.wallet,
            labels,
            extra_seeds: setup.extra_seeds,
        })
    }

    /// List labels published for this passkey's identity. Pass-through
    /// to [`Passkey::list_labels`] (one PRF call to seed the identity
    /// cache, then free for subsequent calls on the same instance).
    pub async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
        self.passkey.list_labels().await
    }

    /// Idempotently publish `label`. Pass-through to
    /// [`Passkey::store_label`].
    pub async fn store_label(&self, label: String) -> Result<(), PasskeyError> {
        self.passkey.store_label(label).await
    }

    /// Pass-through to [`Passkey::is_available`].
    pub async fn is_available(&self) -> Result<bool, PasskeyError> {
        self.passkey.is_available().await
    }
}

/// Convenience constructors that don't cross the `UniFFI` boundary.
impl PasskeyClient {
    /// Build from the SDK's [`crate::Config`], reusing its `api_key`
    /// for the default Nostr-backed label store.
    pub fn from_config(prf_provider: Arc<dyn PrfProvider>, config: &crate::Config) -> Self {
        let passkey = Passkey::from_config(Arc::clone(&prf_provider), config);
        Self {
            passkey,
            prf_provider,
        }
    }

    /// Build with a caller-supplied [`LabelStore`] (server-mediated,
    /// in-memory tests, etc). Rust-only; `UniFFI` bindings see only
    /// [`Self::new`].
    pub fn with_label_store(
        prf_provider: Arc<dyn PrfProvider>,
        label_store: Arc<dyn LabelStore>,
    ) -> Self {
        let passkey = Passkey::with_label_store(Arc::clone(&prf_provider), label_store);
        Self {
            passkey,
            prf_provider,
        }
    }

    /// Access the underlying [`Passkey`] for low-level operations not
    /// covered by the higher-level flows (custom orchestration,
    /// migrations, diagnostics).
    #[must_use]
    pub fn passkey(&self) -> &Passkey {
        &self.passkey
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::super::error::PrfProviderError;

    /// Salt-aware mock that produces deterministic per-salt PRF
    /// outputs so multi-salt ceremonies can round-trip through tests.
    /// Also tracks `create_passkey` calls so registration flows can be
    /// asserted on.
    struct MockProvider {
        base: [u8; 32],
        salts_seen: Mutex<HashMap<String, Vec<u8>>>,
        create_calls: Mutex<usize>,
        fail_create: bool,
    }

    impl MockProvider {
        fn new(base: [u8; 32]) -> Self {
            Self {
                base,
                salts_seen: Mutex::new(HashMap::new()),
                create_calls: Mutex::new(0),
                fail_create: false,
            }
        }

        fn unsupported() -> Self {
            Self {
                fail_create: true,
                ..Self::new([0u8; 32])
            }
        }

        fn output_for(&self, salt: &str) -> Vec<u8> {
            let mut cache = self.salts_seen.lock().unwrap();
            if let Some(v) = cache.get(salt) {
                return v.clone();
            }
            let mut out = [0u8; 32];
            for (i, b) in salt.bytes().enumerate() {
                out[i % 32] ^= b;
            }
            for (i, byte) in out.iter_mut().enumerate() {
                *byte ^= self.base[i];
            }
            let v = out.to_vec();
            cache.insert(salt.to_string(), v.clone());
            v
        }
    }

    #[macros::async_trait]
    impl PrfProvider for MockProvider {
        async fn derive_seed(&self, salt: String) -> Result<Vec<u8>, PrfProviderError> {
            Ok(self.output_for(&salt))
        }

        async fn is_supported(&self) -> Result<bool, PrfProviderError> {
            Ok(true)
        }

        async fn create_passkey(
            &self,
            _request: CreatePasskeyRequest,
        ) -> Result<RegisteredCredential, PrfProviderError> {
            if self.fail_create {
                return Err(PrfProviderError::PrfNotSupported);
            }
            *self.create_calls.lock().unwrap() += 1;
            Ok(RegisteredCredential {
                credential_id: vec![0xab, 0xcd, 0xef],
                aaguid: Some(vec![0; 16]),
                backup_eligible: Some(true),
            })
        }
    }

    #[macros::async_test_all]
    async fn register_returns_credential_and_publishes_label() {
        let provider = Arc::new(MockProvider::new([7u8; 32]));
        let client = PasskeyClient::new(provider.clone(), None);
        let response = client
            .register(RegisterRequest {
                label: Some("alice".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(response.credential.credential_id, vec![0xab, 0xcd, 0xef]);
        assert_eq!(*provider.create_calls.lock().unwrap(), 1);
        assert_eq!(response.wallet.label, "alice");
    }

    #[macros::async_test_all]
    async fn register_propagates_create_passkey_failure() {
        let provider = Arc::new(MockProvider::unsupported());
        let client = PasskeyClient::new(provider, None);
        let result = client.register(RegisterRequest::default()).await;
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::Prf(PrfProviderError::PrfNotSupported)
        ));
    }

    #[macros::async_test_all]
    async fn sign_in_fast_path_returns_wallet_without_listing() {
        let provider = Arc::new(MockProvider::new([0u8; 32]));
        let client = PasskeyClient::new(provider.clone(), None);
        let response = client
            .sign_in(SignInRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(response.wallet.label, "personal");
        // create_passkey is NOT called on the sign-in path.
        assert_eq!(*provider.create_calls.lock().unwrap(), 0);
        // Fast path: no label-store query, so labels stays empty.
        assert!(response.labels.is_empty());
    }

    #[macros::async_test_all]
    async fn sign_in_propagates_extra_seeds() {
        let provider = Arc::new(MockProvider::new([0u8; 32]));
        let client = PasskeyClient::new(provider, None);
        let response = client
            .sign_in(SignInRequest {
                label: None,
                extra_salts: vec![NamedSalt {
                    name: "db_key".to_string(),
                }],
            })
            .await
            .unwrap();
        assert_eq!(response.extra_seeds.len(), 1);
        assert!(response.extra_seeds.contains_key("db_key"));
    }
}
