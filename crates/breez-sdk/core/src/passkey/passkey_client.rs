//! High-level passkey orchestration. [`PasskeyClient`] is the ergonomic
//! entry point for hosts: it composes the lower-level [`Passkey`]
//! (label store + identity cache) and the [`PrfProvider`] trait into a
//! handful of named flows that match real onboarding UI states.

use std::sync::Arc;

use super::Passkey;
use super::error::{PasskeyError, PrfProviderError};
use super::models::{PasskeyConfig, RegisteredCredential, SetupWalletRequest, Wallet};
use super::passkey_prf_provider::PrfProvider;

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

    /// A list of already-registered credential IDs. Prevents
    /// registering the same device twice: when any entry matches a
    /// credential already on the device, the platform raises
    /// [`crate::passkey::PrfProviderError::CredentialAlreadyExists`]
    /// so the host can flip the user to the sign-in path. Forwarded
    /// to [`PrfProvider::create_passkey`].
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub exclude_credentials: Vec<Vec<u8>>,
}

/// Response from [`PasskeyClient::register`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RegisterResponse {
    /// The newly-derived wallet for [`RegisterRequest::label`].
    pub wallet: Wallet,
    /// Metadata for the credential the platform just registered. Hosts
    /// SHOULD persist [`RegisteredCredential::credential_id`] so they
    /// can populate `exclude_credentials` on future
    /// [`PasskeyClient::register`] calls.
    pub credential: RegisteredCredential,
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

    /// A list of credential IDs the assertion is restricted to. The
    /// primary use case is reauthentication when the user is already
    /// known: if any of the listed credentials is available locally,
    /// the OS prompts for device unlock straight away (no account
    /// picker); otherwise the user is asked to present another
    /// device (paired phone or security key) that holds a valid
    /// credential. Empty / omitted lets the OS pick any matching
    /// credential for this RP. Forwarded to
    /// [`crate::passkey::DeriveSeedsRequest::allow_credentials`].
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credentials: Vec<Vec<u8>>,

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
    /// The credential ID the user used for this sign-in, when the
    /// underlying [`PrfProvider`] surfaces it. `None` for providers
    /// that don't expose this signal (CLI / file-backed / hardware
    /// providers).
    pub credential_id: Option<Vec<u8>>,
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

    /// A list of credential IDs to restrict the silent sign-in
    /// attempt to (reauthentication path). See
    /// [`SignInRequest::allow_credentials`]. Ignored on the fallback
    /// registration path.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub allow_credentials: Vec<Vec<u8>>,

    /// A list of already-registered credential IDs to surface
    /// duplicates on the fallback registration path. See
    /// [`RegisterRequest::exclude_credentials`]. Ignored on the
    /// silent sign-in attempt.
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub exclude_credentials: Vec<Vec<u8>>,
}

/// Response from [`PasskeyClient::connect_with_passkey`].
///
/// `registered_credential` doubles as the path discriminator:
/// - When present, a new credential was registered (silent sign-in
///   fast-failed with `CredentialNotFound`). Persist
///   `credential.credential_id` in your `excludeCredentials` set.
/// - When absent, silent sign-in succeeded for an existing
///   credential. Hosts that need the asserted credential ID per
///   sign-in (e.g. to mark "recently used" in a `CredentialRegistry`)
///   should use [`PasskeyClient::sign_in`] directly; the unified
///   call doesn't surface it.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectWithPasskeyResponse {
    pub wallet: Wallet,
    pub registered_credential: Option<RegisteredCredential>,
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
/// Label and credential management hang off the [`Self::labels`] and
/// [`Self::credentials`] sub-objects.
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
            .create_passkey(request.exclude_credentials)
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
            credential,
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
                allow_credentials: request.allow_credentials,
                prefer_immediately_available_credentials: request
                    .prefer_immediately_available_credentials,
            })
            .await?;

        // The credential ID observed during the derive ceremony, carried
        // back on the setup result.
        let credential_id = setup.credential_id.clone();

        let labels = if discovery {
            self.passkey.list_labels().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(SignInResponse {
            wallet: setup.wallet,
            labels,
            credential_id,
        })
    }

    /// Single-CTA onboarding: silent sign-in, falling through to
    /// registration when no credential exists on the device. The
    /// returned [`ConnectFlow`] tells the caller which path ran.
    ///
    /// The silent sign-in attempt pins
    /// `prefer_immediately_available_credentials = true` regardless of
    /// what [`SignInRequest`] would carry: the fallback contract
    /// depends on the OS fast-failing with [`PrfProviderError::CredentialNotFound`]
    /// when no local credential exists. Without the fast-fail, the
    /// OS would show the cross-device picker and a user dismiss would
    /// surface as a real `Cancel`, which is propagated unchanged
    /// rather than triggering registration. All other errors
    /// (`Cancel`, `Timeout`, `Configuration`, etc.) propagate
    /// unchanged: only `CredentialNotFound` flips to the register
    /// path.
    ///
    /// Mobile-only: meant for iOS 18+ / Android 9+ where
    /// `preferImmediatelyAvailableCredentials` is honored. The web
    /// equivalent (`mediation: 'immediate'` / `uiMode: 'immediate'`)
    /// is not yet stable cross-browser, so this method is not
    /// surfaced on WASM. Hosts on web should call
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
                registered_credential: None,
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
                    registered_credential: Some(register_response.credential),
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

    /// Credential sub-object. Inspect or mutate the provider's
    /// persisted credential-ID set.
    pub fn credentials(&self) -> Arc<PasskeyCredentials> {
        Arc::new(PasskeyCredentials {
            prf_provider: Arc::clone(self.passkey.prf_provider()),
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

/// Credential sub-object surfaced from [`PasskeyClient::credentials`].
/// Reads / mutates the provider's persisted credential-ID set; methods
/// no-op on providers without a registry (CLI / file / `YubiKey`).
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PasskeyCredentials {
    prf_provider: Arc<dyn PrfProvider>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl PasskeyCredentials {
    /// Read the persisted set of credential IDs for the current RP.
    pub async fn get(&self) -> Result<Vec<Vec<u8>>, PasskeyError> {
        Ok(self.prf_provider.get_known_credential_ids().await?)
    }

    /// Drop a single credential ID from the persisted set.
    pub async fn remove(&self, credential_id: Vec<u8>) -> Result<(), PasskeyError> {
        Ok(self
            .prf_provider
            .remove_known_credential_id(credential_id)
            .await?)
    }

    /// Clear the persisted credential-ID set for the current RP.
    pub async fn clear(&self) -> Result<(), PasskeyError> {
        Ok(self.prf_provider.clear_known_credential_ids().await?)
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
    }

    impl MockProvider {
        fn new(base: [u8; 32]) -> Self {
            Self {
                base,
                salts_seen: Mutex::new(HashMap::new()),
                create_calls: Mutex::new(0),
                fail_create: false,
                derive_errors: Mutex::new(Vec::new()),
            }
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
                credential_id: None,
            })
        }

        async fn is_supported(&self) -> Result<bool, PrfProviderError> {
            Ok(true)
        }

        async fn create_passkey(
            &self,
            _exclude_credentials: Vec<Vec<u8>>,
        ) -> Result<RegisteredCredential, PrfProviderError> {
            if self.fail_create {
                return Err(PrfProviderError::PrfNotSupported);
            }
            let mut count = self.create_calls.lock().unwrap();
            *count = count.checked_add(1).expect("create_calls overflow");
            Ok(RegisteredCredential {
                credential_id: vec![0xab, 0xcd, 0xef],
                user_id: vec![0u8; 16],
                aaguid: Some(vec![0; 16]),
                backup_eligible: Some(true),
            })
        }
    }

    #[macros::async_test_all]
    async fn register_returns_credential_and_publishes_label() {
        let provider = Arc::new(MockProvider::new([7u8; 32]));
        let client = PasskeyClient::new(provider.clone(), None, None);
        let response = client
            .register(RegisterRequest {
                label: Some("alice".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(response.credential.credential_id, vec![0xab, 0xcd, 0xef]);
        assert_eq!(response.credential.user_id.len(), 16);
        assert_eq!(*provider.create_calls.lock().unwrap(), 1);
        assert_eq!(response.wallet.label, "alice");
    }

    #[macros::async_test_all]
    async fn register_propagates_create_passkey_failure() {
        let provider = Arc::new(MockProvider::unsupported());
        let client = PasskeyClient::new(provider, None, None);
        let result = client.register(RegisterRequest::default()).await;
        assert!(matches!(
            result.unwrap_err(),
            PasskeyError::Prf(PrfProviderError::PrfNotSupported)
        ));
    }

    #[macros::async_test_all]
    async fn sign_in_fast_path_returns_wallet_without_listing() {
        let provider = Arc::new(MockProvider::new([0u8; 32]));
        let client = PasskeyClient::new(provider.clone(), None, None);
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
    async fn default_label_from_config_overrides_internal_default() {
        let provider = Arc::new(MockProvider::new([0u8; 32]));
        let client = PasskeyClient::new(
            provider,
            None,
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
    }

    #[macros::async_test_all]
    async fn connect_with_passkey_returns_none_credential_when_sign_in_succeeds() {
        let provider = Arc::new(MockProvider::new([1u8; 32]));
        let client = PasskeyClient::new(provider.clone(), None, None);
        let response = client
            .connect_with_passkey(ConnectWithPasskeyRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(response.wallet.label, "personal");
        // Sign-in path: no new credential to surface.
        assert!(response.registered_credential.is_none());
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
        let client = PasskeyClient::new(provider.clone(), None, None);
        let response = client
            .connect_with_passkey(ConnectWithPasskeyRequest {
                label: Some("personal".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(response.wallet.label, "personal");
        let credential = response
            .registered_credential
            .expect("registered path must surface the new credential");
        assert_eq!(credential.credential_id, vec![0xab, 0xcd, 0xef]);
        assert_eq!(*provider.create_calls.lock().unwrap(), 1);
    }

    #[macros::async_test_all]
    async fn connect_with_passkey_propagates_cancel_without_registering() {
        let provider = Arc::new(MockProvider::new([3u8; 32]));
        provider.queue_derive_error(PrfProviderError::UserCancelled);
        let client = PasskeyClient::new(provider.clone(), None, None);
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
