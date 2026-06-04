//! High-level passkey orchestration. [`PasskeyClient`] is the ergonomic
//! entry point for hosts: it composes the lower-level [`Passkey`]
//! (label store + identity cache) and the [`PrfProvider`] trait into a
//! handful of named flows that match real onboarding UI states.

use std::sync::Arc;

use super::Passkey;
use super::error::{PasskeyError, PrfProviderError};
use super::models::{PasskeyConfig, PasskeyCredential, SetupWalletRequest, Wallet};
use super::passkey_prf_provider::PrfProvider;
#[cfg(test)]
use super::{LabelStore, LabelStoreBuilder};

/// Single-value result of [`PasskeyClient::check_availability`].
/// Collapses [`PrfProvider::is_supported`] +
/// [`PrfProvider::check_domain_association`] into one variant per
/// distinct host UX reaction.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PasskeyAvailability {
    /// PRF is supported and the platform's domain-association check
    /// (when present) passed. Safe to proceed with register / sign-in.
    Available,
    /// The authenticator does not implement the `WebAuthn` PRF
    /// extension. Hosts gate the passkey UX path off this value.
    PrfUnsupported,
    /// PRF is supported but the platform's out-of-band verification
    /// (iOS AASA / Android assetlinks / browser `rpId` scope) reports a
    /// configuration mismatch. The strings carry the verification
    /// origin and the concrete reason for diagnostic UI.
    NotAssociated { source: String, reason: String },
    /// Domain-association verification was not performed (no source,
    /// SSR context, etc.). Not a negative signal; passkey flows are
    /// still safe to attempt.
    Skipped { reason: String },
}

/// Request shape for [`PasskeyClient::register`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterRequest {
    /// User-chosen label for the new wallet. Defaults to the configured
    /// default label when `None`. Always published to the label
    /// store as part of registration.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,

    /// Optional list of already-registered credential IDs. Prevents
    /// registering the same device twice: when any entry matches a
    /// credential already on the device, the platform raises
    /// [`crate::passkey::PrfProviderError::CredentialAlreadyExists`]
    /// so the host can flip the user to the sign-in path. Unset is
    /// treated as empty. Forwarded to [`PrfProvider::create_passkey`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub exclude_credentials: Option<Vec<Vec<u8>>>,
}

/// Response from [`PasskeyClient::register`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterResponse {
    /// The newly-derived wallet for [`RegisterRequest::label`].
    pub wallet: Wallet,
    /// The credential the platform just registered. Persist
    /// [`PasskeyCredential::credential_id`] to populate
    /// `exclude_credentials` on future [`PasskeyClient::register`]
    /// calls. Always set on the register path.
    pub credential: Option<PasskeyCredential>,
}

/// Request shape for [`PasskeyClient::sign_in`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SignInRequest {
    /// When present, the fast path: one ceremony, no label-store
    /// query. When absent, triggers discovery: derives the configured
    /// default label and also returns the user's full label set in
    /// [`SignInResponse::labels`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,

    /// Optional credential IDs the assertion is restricted to
    /// (reauthentication of a known user). Unset or empty lets the OS
    /// pick any matching credential for this RP. Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::allow_credentials`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub allow_credentials: Option<Vec<Vec<u8>>>,

    /// Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::prefer_immediately_available_credentials`].
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub prefer_immediately_available_credentials: Option<bool>,
}

/// Response from [`PasskeyClient::sign_in`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SignInResponse {
    pub wallet: Wallet,
    /// Empty on the fast path. Populated on discovery (or empty if
    /// the label store was unreachable).
    pub labels: Vec<String>,
    /// The credential the user signed in with, when the underlying
    /// [`PrfProvider`] surfaces it. `None` for providers that don't
    /// expose this signal (CLI / file-backed / hardware). Only
    /// `credential_id` is set: a sign-in assertion carries no
    /// attestation.
    pub credential: Option<PasskeyCredential>,
}

/// Request shape for [`PasskeyClient::connect_with_passkey`].
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectWithPasskeyRequest {
    /// Wallet label. Defaults to the configured default label when
    /// `None`. Used both for the silent sign-in attempt and, if it
    /// fast-fails, for the fallback registration.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub label: Option<String>,

    /// Optional credential IDs to restrict the silent sign-in
    /// attempt to (reauthentication path). See
    /// [`SignInRequest::allow_credentials`]. Ignored on the fallback
    /// registration path.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub allow_credentials: Option<Vec<Vec<u8>>>,

    /// Optional already-registered credential IDs to surface
    /// duplicates on the fallback registration path. See
    /// [`RegisterRequest::exclude_credentials`]. Ignored on the
    /// silent sign-in attempt.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub exclude_credentials: Option<Vec<Vec<u8>>>,
}

/// Response from [`PasskeyClient::connect_with_passkey`].
///
/// `credential` carries whichever credential signed in or was
/// registered, when the provider surfaces it. The register path also
/// populates the attestation fields (`aaguid`, `backup_eligible`); the
/// sign-in path sets only `credential_id`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectWithPasskeyResponse {
    pub wallet: Wallet,
    pub credential: Option<PasskeyCredential>,
}

/// High-level orchestration over a [`PrfProvider`] and the internal
/// Nostr-backed label store. Two named flows match the real onboarding
/// states:
///
/// - [`Self::register`]: first-time setup (create credential + derive
///   wallet + publish label) in one ceremony where the platform
///   supports dual-salt PRF.
/// - [`Self::sign_in`]: returning user. Fast path when the host has
///   the label cached locally; cold-restore-with-discovery when not.
///
/// Label management hangs off the [`Self::labels`] sub-object.
///
/// The `breez_api_key` is the Breez relay key used for authenticated
/// (NIP-42) label storage. Hosts that already construct the SDK
/// [`crate::Config`] can use [`Self::from_config`] to forward it.
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PasskeyClient {
    passkey: Passkey,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl PasskeyClient {
    /// Construct with the default Nostr-backed label store.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    pub fn new(
        prf_provider: Arc<dyn PrfProvider>,
        breez_api_key: Option<String>,
        config: Option<PasskeyConfig>,
    ) -> Self {
        Self {
            passkey: Passkey::new(prf_provider, breez_api_key, config),
        }
    }

    /// One-shot capability + configuration probe. Collapses
    /// [`PrfProvider::is_supported`] and
    /// [`PrfProvider::check_domain_association`] into a single value
    /// hosts can branch on.
    pub async fn check_availability(&self) -> Result<PasskeyAvailability, PasskeyError> {
        self.passkey.check_availability().await
    }

    /// First-time setup. Drives [`PrfProvider::create_passkey`] (one
    /// ceremony) followed by the wallet-derivation flow that backs
    /// [`Passkey::setup_wallet`] (one ceremony, dual-salt where
    /// supported). The label is always published on success.
    pub async fn register(
        &self,
        request: RegisterRequest,
    ) -> Result<RegisterResponse, PasskeyError> {
        let credential = self
            .passkey
            .prf_provider()
            .create_passkey(request.exclude_credentials.unwrap_or_default())
            .await?;

        let setup = self
            .passkey
            .setup_wallet(SetupWalletRequest {
                label: request.label,
                publish_label: true,
                // Registration always derives via the just-created
                // credential; callers don't drive sign-in pinning here.
                allow_credentials: Vec::new(),
                prefer_immediately_available_credentials: None,
            })
            .await?;

        Ok(RegisterResponse {
            wallet: setup.wallet,
            credential: Some(credential),
        })
    }

    /// Returning-user sign-in. Fast path (`label` set) skips the
    /// label-store query; discovery path (`label = None`) derives
    /// the configured default label and lists the user's labels in
    /// the same ceremony. Never re-publishes the label.
    pub async fn sign_in(&self, request: SignInRequest) -> Result<SignInResponse, PasskeyError> {
        let discovery = request.label.is_none();

        let setup = self
            .passkey
            .setup_wallet(SetupWalletRequest {
                label: request.label,
                publish_label: false,
                allow_credentials: request.allow_credentials.unwrap_or_default(),
                prefer_immediately_available_credentials: request
                    .prefer_immediately_available_credentials,
            })
            .await?;

        // The credential observed during the derive ceremony, carried
        // back on the setup result. A sign-in assertion yields only the
        // credential ID, no attestation.
        let credential = setup
            .credential_id
            .clone()
            .map(PasskeyCredential::from_credential_id);

        let labels = if discovery {
            self.passkey.list_labels().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(SignInResponse {
            wallet: setup.wallet,
            labels,
            credential,
        })
    }

    /// Single-CTA onboarding: silent sign-in, falling through to
    /// registration when no credential exists on the device. The returned
    /// [`ConnectFlow`] tells the caller which path ran.
    ///
    /// The silent sign-in pins `prefer_immediately_available_credentials =
    /// true` regardless of [`SignInRequest`]: the fallback depends on the OS
    /// fast-failing with [`PrfProviderError::CredentialNotFound`] when no
    /// local credential exists. Only `CredentialNotFound` flips to the
    /// register path; every other error (`Cancel`, `Timeout`, ...) propagates
    /// unchanged.
    ///
    /// Mobile-only: meant for iOS 18+ / Android 9+ where
    /// `preferImmediatelyAvailableCredentials` is honored. The web
    /// equivalent (`mediation: 'immediate'`) is not yet stable
    /// cross-browser, so this is not surfaced on WASM; web hosts call
    /// [`Self::sign_in`] and catch `CredentialNotFound` themselves.
    pub async fn connect_with_passkey(
        &self,
        request: ConnectWithPasskeyRequest,
    ) -> Result<ConnectWithPasskeyResponse, PasskeyError> {
        let sign_in_result = self
            .sign_in(SignInRequest {
                label: request.label.clone(),
                allow_credentials: request.allow_credentials,
                prefer_immediately_available_credentials: Some(true),
            })
            .await;

        match sign_in_result {
            Ok(response) => Ok(ConnectWithPasskeyResponse {
                wallet: response.wallet,
                credential: response.credential,
            }),
            Err(PasskeyError::Prf(PrfProviderError::CredentialNotFound(_))) => {
                let register_response = self
                    .register(RegisterRequest {
                        label: request.label,
                        exclude_credentials: request.exclude_credentials,
                    })
                    .await?;
                Ok(ConnectWithPasskeyResponse {
                    wallet: register_response.wallet,
                    credential: register_response.credential,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Label sub-object. List or publish labels for this passkey's
    /// identity.
    pub fn labels(&self) -> Arc<PasskeyLabels> {
        Arc::new(PasskeyLabels {
            passkey: self.passkey.clone(),
        })
    }
}

/// Convenience constructors that don't cross the `UniFFI` boundary.
impl PasskeyClient {
    /// Build from the SDK's [`crate::Config`], reusing its `api_key`
    /// for the default Nostr-backed label store.
    pub fn from_config(
        prf_provider: Arc<dyn PrfProvider>,
        sdk_config: &crate::Config,
        passkey_config: Option<PasskeyConfig>,
    ) -> Self {
        Self::new(prf_provider, sdk_config.api_key.clone(), passkey_config)
    }

    /// Test-only: construct with a custom [`LabelStore`] builder so unit
    /// tests inject an in-memory store instead of reaching Nostr relays.
    #[cfg(test)]
    pub(crate) fn new_with_store_builder(
        prf_provider: Arc<dyn PrfProvider>,
        breez_api_key: Option<String>,
        config: Option<PasskeyConfig>,
        store_builder: LabelStoreBuilder,
    ) -> Self {
        Self {
            passkey: Passkey::new_with_store_builder(
                prf_provider,
                breez_api_key,
                config,
                store_builder,
            ),
        }
    }
}

/// Label sub-object surfaced from [`PasskeyClient::labels`]. Holds a
/// clone of the parent [`Passkey`] so calls re-use its cached identity.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PasskeyLabels {
    passkey: Passkey,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl PasskeyLabels {
    /// List labels published for this passkey's identity.
    pub async fn list(&self) -> Result<Vec<String>, PasskeyError> {
        self.passkey.list_labels().await
    }

    /// Idempotently publish `label` for this passkey's identity.
    pub async fn store(&self, label: String) -> Result<(), PasskeyError> {
        self.passkey.store_label(label).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::super::error::PrfProviderError;
    use super::super::{DeriveSeedsOutput, DeriveSeedsRequest};

    /// Salt-aware mock that produces deterministic per-salt PRF
    /// outputs so multi-salt ceremonies can round-trip through tests.
    /// Also tracks `create_passkey` calls so registration flows can be
    /// asserted on.
    struct MockProvider {
        base: [u8; 32],
        salts_seen: Mutex<HashMap<String, Vec<u8>>>,
        create_calls: Mutex<usize>,
        fail_create: bool,
        /// FIFO of errors to return from `derive_seeds`. Each call pops
        /// the front; when empty, `derive_seeds` succeeds.
        derive_errors: Mutex<Vec<PrfProviderError>>,
        /// Credential ID surfaced from `derive_seeds`, mimicking a
        /// provider that reports the signed-in credential. `None` mirrors
        /// providers that don't expose it (CLI / file-backed / hardware).
        derive_credential_id: Option<Vec<u8>>,
    }

    impl MockProvider {
        fn new(base: [u8; 32]) -> Self {
            Self {
                base,
                salts_seen: Mutex::new(HashMap::new()),
                create_calls: Mutex::new(0),
                fail_create: false,
                derive_errors: Mutex::new(Vec::new()),
                derive_credential_id: None,
            }
        }

        /// Surface `credential_id` from `derive_seeds`, mimicking a
        /// sign-in assertion that reports which credential the user picked.
        fn with_derive_credential_id(mut self, credential_id: Vec<u8>) -> Self {
            self.derive_credential_id = Some(credential_id);
            self
        }

        fn unsupported() -> Self {
            Self {
                fail_create: true,
                ..Self::new([0u8; 32])
            }
        }

        fn queue_derive_error(&self, err: PrfProviderError) {
            self.derive_errors.lock().unwrap().push(err);
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
        async fn derive_seeds(
            &self,
            request: DeriveSeedsRequest,
        ) -> Result<DeriveSeedsOutput, PrfProviderError> {
            if let Some(err) = {
                let mut errs = self.derive_errors.lock().unwrap();
                if errs.is_empty() {
                    None
                } else {
                    Some(errs.remove(0))
                }
            } {
                return Err(err);
            }
            Ok(DeriveSeedsOutput {
                seeds: request
                    .salts
                    .into_iter()
                    .map(|s| self.output_for(&s))
                    .collect(),
                credential_id: self.derive_credential_id.clone(),
            })
        }

        async fn is_supported(&self) -> Result<bool, PrfProviderError> {
            Ok(true)
        }

        async fn create_passkey(
            &self,
            _exclude_credentials: Vec<Vec<u8>>,
        ) -> Result<PasskeyCredential, PrfProviderError> {
            if self.fail_create {
                return Err(PrfProviderError::PrfNotSupported);
            }
            let mut count = self.create_calls.lock().unwrap();
            *count = count.checked_add(1).expect("create_calls overflow");
            Ok(PasskeyCredential {
                credential_id: vec![0xab, 0xcd, 0xef],
                user_id: Some(vec![0u8; 16]),
                aaguid: Some(vec![0; 16]),
                backup_eligible: Some(true),
            })
        }
    }

    /// Records the calls the orchestrator makes against the label store.
    #[derive(Default)]
    struct StoreCalls {
        /// Labels handed to `store_label`, in order.
        stored: Vec<String>,
        /// Number of `list_labels` queries.
        list_calls: usize,
    }

    /// In-memory [`LabelStore`] so client unit tests never reach the Nostr
    /// relays. Records every call into a shared [`StoreCalls`] so tests can
    /// assert the publish / query contract, not just that nothing panicked.
    struct MockLabelStore {
        keys: nostr::Keys,
        calls: Arc<Mutex<StoreCalls>>,
    }

    #[macros::async_trait]
    impl LabelStore for MockLabelStore {
        async fn store_label(&self, label: &str) -> Result<(), PasskeyError> {
            self.calls.lock().unwrap().stored.push(label.to_string());
            Ok(())
        }

        async fn list_labels(&self) -> Result<Vec<String>, PasskeyError> {
            let mut calls = self.calls.lock().unwrap();
            calls.list_calls = calls
                .list_calls
                .checked_add(1)
                .expect("list_calls overflow");
            Ok(Vec::new())
        }

        fn signing_keys(&self) -> nostr::Keys {
            self.keys.clone()
        }
    }

    /// A [`PasskeyClient`] backed by the in-memory label store (no network),
    /// plus the shared [`StoreCalls`] recording what the client published or
    /// queried. Every store this client builds shares the one recorder.
    fn client_with_store(
        prf_provider: Arc<dyn PrfProvider>,
        config: Option<PasskeyConfig>,
    ) -> (PasskeyClient, Arc<Mutex<StoreCalls>>) {
        let calls = Arc::new(Mutex::new(StoreCalls::default()));
        let store_calls = calls.clone();
        let builder: LabelStoreBuilder = Arc::new(move |keys, _api_key| {
            Arc::new(MockLabelStore {
                keys,
                calls: store_calls.clone(),
            }) as Arc<dyn LabelStore>
        });
        let client = PasskeyClient::new_with_store_builder(prf_provider, None, config, builder);
        (client, calls)
    }

    /// Store-agnostic [`client_with_store`] for tests that don't inspect
    /// label persistence.
    fn test_client(
        prf_provider: Arc<dyn PrfProvider>,
        config: Option<PasskeyConfig>,
    ) -> PasskeyClient {
        client_with_store(prf_provider, config).0
    }

    #[macros::async_test_all]
    async fn register_returns_credential_and_publishes_label() {
        let provider = Arc::new(MockProvider::new([7u8; 32]));
        let (client, store) = client_with_store(provider.clone(), None);
        let response = client
            .register(RegisterRequest {
                label: Some("alice".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        let credential = response
            .credential
            .expect("register surfaces the credential");
        assert_eq!(credential.credential_id, vec![0xab, 0xcd, 0xef]);
        assert_eq!(credential.user_id.expect("user_id").len(), 16);
        // Registration is the one ceremony that carries attestation, so
        // the full credential shape must round-trip (hosts key UI off it:
        // aaguid -> provider name, backup_eligible -> sync status).
        assert_eq!(credential.aaguid.expect("aaguid").len(), 16);
        assert_eq!(credential.backup_eligible, Some(true));
        assert_eq!(*provider.create_calls.lock().unwrap(), 1);
        assert_eq!(response.wallet.label, "alice");
        // Registration publishes the label to the store exactly once.
        assert_eq!(store.lock().unwrap().stored, vec!["alice".to_string()]);
    }

    #[macros::async_test_all]
    async fn register_propagates_create_passkey_failure() {
        let provider = Arc::new(MockProvider::unsupported());
        let client = test_client(provider, None);
        let result = client.register(RegisterRequest::default()).await;
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::Prf(PrfProviderError::PrfNotSupported)
        ));
    }

    #[macros::async_test_all]
    async fn sign_in_fast_path_returns_wallet_without_listing() {
        let provider = Arc::new(MockProvider::new([0u8; 32]));
        let (client, store) = client_with_store(provider.clone(), None);
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
        // Fast path: the store is never queried (the reason `labels` is
        // empty), and a known label is never re-published.
        let calls = store.lock().unwrap();
        assert_eq!(calls.list_calls, 0);
        assert!(calls.stored.is_empty());
        assert!(response.labels.is_empty());
    }

    #[macros::async_test_all]
    async fn sign_in_surfaces_credential_id_without_attestation() {
        // Provider reports the signed-in credential; a sign-in assertion
        // carries no attestation, so only credential_id is populated.
        let provider = Arc::new(
            MockProvider::new([0u8; 32]).with_derive_credential_id(vec![0x01, 0x02, 0x03]),
        );
        let client = test_client(provider, None);
        let response = client
            .sign_in(SignInRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let credential = response
            .credential
            .expect("sign-in surfaces the asserted credential");
        assert_eq!(credential.credential_id, vec![0x01, 0x02, 0x03]);
        assert!(credential.user_id.is_none());
        assert!(credential.aaguid.is_none());
        assert!(credential.backup_eligible.is_none());
    }

    #[macros::async_test_all]
    async fn default_label_from_config_overrides_internal_default() {
        let provider = Arc::new(MockProvider::new([0u8; 32]));
        let (client, store) = client_with_store(
            provider,
            Some(PasskeyConfig {
                default_label: Some("my-app".to_string()),
                ..Default::default()
            }),
        );
        let response = client
            .sign_in(SignInRequest {
                label: None,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(response.wallet.label, "my-app");
        // Discovery (label = None) queries the store for the label set;
        // the positive control for the fast path's zero-query assertion.
        assert_eq!(store.lock().unwrap().list_calls, 1);
    }

    #[macros::async_test_all]
    async fn connect_with_passkey_returns_none_credential_when_sign_in_succeeds() {
        let provider = Arc::new(MockProvider::new([1u8; 32]));
        let client = test_client(provider.clone(), None);
        let response = client
            .connect_with_passkey(ConnectWithPasskeyRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(response.wallet.label, "personal");
        // Sign-in path: the mock provider surfaces no credential_id.
        assert!(response.credential.is_none());
        // No registration ceremony on the silent-sign-in success path.
        assert_eq!(*provider.create_calls.lock().unwrap(), 0);
    }

    #[macros::async_test_all]
    async fn connect_with_passkey_falls_through_to_register_on_no_credential() {
        let provider = Arc::new(MockProvider::new([2u8; 32]));
        // Silent sign-in attempt fast-fails; subsequent derive (called
        // from register's setup_wallet) succeeds.
        provider.queue_derive_error(PrfProviderError::CredentialNotFound(
            "no local credential".to_string(),
        ));
        let client = test_client(provider.clone(), None);
        let response = client
            .connect_with_passkey(ConnectWithPasskeyRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(response.wallet.label, "personal");
        let credential = response
            .credential
            .expect("registered path must surface the new credential");
        assert_eq!(credential.credential_id, vec![0xab, 0xcd, 0xef]);
        assert_eq!(*provider.create_calls.lock().unwrap(), 1);
    }

    #[macros::async_test_all]
    async fn connect_with_passkey_propagates_cancel_without_registering() {
        let provider = Arc::new(MockProvider::new([3u8; 32]));
        provider.queue_derive_error(PrfProviderError::UserCancelled);
        let client = test_client(provider.clone(), None);
        let result = client
            .connect_with_passkey(ConnectWithPasskeyRequest::default())
            .await;
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::Prf(PrfProviderError::UserCancelled)
        ));
        // A real cancel must NOT silently register.
        assert_eq!(*provider.create_calls.lock().unwrap(), 0);
    }
}
