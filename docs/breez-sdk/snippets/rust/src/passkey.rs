use anyhow::Result;
use breez_sdk_spark::passkey::{
    DeriveSeedsRequest, DomainAssociation, PasskeyAvailability, PasskeyClient, PasskeyConfig,
    PasskeyError, PrfProvider, PrfProviderError, RegisterRequest, RegisteredCredential,
    SignInRequest, SignInResponse, Wallet,
};
use breez_sdk_spark::{ConnectRequest, Network, connect, default_config};
use std::sync::Arc;

// ANCHOR: implement-prf-provider
/// Implement the PrfProvider trait for custom logic if the built-in
/// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
/// air-gapped backup file, etc.). Three required methods: derive_seeds
/// for derivation, is_supported for the capability probe; create_passkey
/// for registration is optional (defaults to PrfNotSupported).
struct CustomPrfProvider;

#[async_trait::async_trait]
impl PrfProvider for CustomPrfProvider {
    async fn derive_seeds(
        &self,
        _request: DeriveSeedsRequest,
    ) -> Result<Vec<Vec<u8>>, PrfProviderError> {
        // Call platform passkey API with PRF extension. Use the dual-salt
        // ceremony when the authenticator supports it (one OS prompt for
        // N salts) and fall back to per-salt assertions otherwise.
        // Returns one 32-byte PRF output per salt in input order.
        todo!("Implement using WebAuthn or native passkey APIs")
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        // Check if a PRF-capable authenticator is reachable from this
        // platform / device.
        todo!("Check platform passkey availability")
    }

    async fn create_passkey(
        &self,
        _exclude_credential_ids: Vec<Vec<u8>>,
    ) -> Result<RegisteredCredential, PrfProviderError> {
        // Register a new credential and return its ID, the WebAuthn
        // user.id the platform recorded (returned for host-side
        // correlation, never host-supplied), AAGUID, and BE flag.
        todo!("Implement registration via WebAuthn create() / native API")
    }

    async fn check_domain_association(&self) -> Result<DomainAssociation, PrfProviderError> {
        // Optional: verify the app's identity against the platform's
        // domain verification source. Custom providers without a
        // verification source return Skipped, which tells callers
        // "proceed with WebAuthn as normal".
        Ok(DomainAssociation::Skipped {
            reason: "CustomPrfProvider does not verify domain association".to_string(),
        })
    }
}
// ANCHOR_END: implement-prf-provider

async fn check_availability() -> Result<()> {
    // ANCHOR: check-availability
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // check_availability collapses is_supported + check_domain_association
    // into a single tagged value. Branch on the variant the host needs.
    match passkey.check_availability().await? {
        PasskeyAvailability::Available => {
            // Show passkey as primary option.
        }
        PasskeyAvailability::PrfUnsupported => {
            // Fall back to mnemonic flow.
        }
        PasskeyAvailability::NotAssociated { source, reason } => {
            eprintln!("Domain association failed (source={source}): {reason}");
        }
        PasskeyAvailability::Skipped { reason: _ } => {
            // No verification source on this platform; proceed normally.
        }
    }
    // ANCHOR_END: check-availability
    Ok(())
}

async fn connect_with_passkey() -> Result<breez_sdk_spark::BreezSdk> {
    // ANCHOR: connect-with-passkey
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // sign_in derives the wallet seed for an existing credential. With
    // bulk PRF on iOS+Android this is a single OS prompt that derives
    // master + label seeds in one ceremony.
    let response = passkey
        .sign_in(SignInRequest {
            label: Some("personal".to_string()),
            ..Default::default()
        })
        .await?;

    let config = default_config(Network::Mainnet);
    let sdk = connect(ConnectRequest {
        config,
        seed: response.wallet.seed,
        storage_dir: "./.data".to_string(),
    })
    .await?;
    // ANCHOR_END: connect-with-passkey
    Ok(sdk)
}

async fn register_new_passkey() -> Result<breez_sdk_spark::BreezSdk> {
    // ANCHOR: register-passkey
    // For a brand-new user with no existing passkey: register() creates
    // the credential AND derives the wallet seed in one orchestrated
    // call. On iOS+Android this is 2 OS prompts total (1 create + 1
    // dual-salt assert) thanks to the SDK's bulk-PRF setup_wallet path.
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    let response = passkey
        .register(RegisterRequest {
            label: Some("personal".to_string()),
            exclude_credential_ids: vec![],
        })
        .await?;

    // Hosts SHOULD persist credential.credential_id (for excludeCredentialIds
    // bookkeeping) and credential.user_id (for server-side correlation).
    // The SDK generates user_id; it is never host-supplied.
    let _persist = (
        response.credential.credential_id.clone(),
        response.credential.user_id.clone(),
    );

    let config = default_config(Network::Mainnet);
    let sdk = connect(ConnectRequest {
        config,
        seed: response.wallet.seed,
        storage_dir: "./.data".to_string(),
    })
    .await?;
    // ANCHOR_END: register-passkey
    Ok(sdk)
}

async fn list_labels() -> Result<Vec<String>> {
    // ANCHOR: list-labels
    let prf_provider = Arc::new(CustomPrfProvider);
    // breez_api_key enables authenticated (NIP-42) access to the Breez
    // relay; pass None for public-relay-only label sync.
    let passkey = PasskeyClient::new(
        prf_provider,
        Some("<breez api key>".to_string()),
        Some(PasskeyConfig {
            // Optional: override the wallet label used when register /
            // sign_in receive `label = None`. Falls back to the SDK's
            // internal "Default" when unset.
            default_label: Some("personal".to_string()),
        }),
    );

    // sign_in with no label runs in discovery mode: it derives the
    // master seed AND lists labels in the same ceremony, so a follow-up
    // labels().list() reads from the cached identity for free.
    let labels = passkey.labels().list().await?;

    for label in &labels {
        println!("Found label: {label}");
    }
    // ANCHOR_END: list-labels
    Ok(labels)
}

async fn store_label() -> Result<()> {
    // ANCHOR: store-label
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None);

    // For a new label on an existing identity, call sign_in(new_label)
    // first to seed the SDK's identity cache via setup_wallet, THEN
    // labels().store() uses the cached identity for free (1 OS prompt total).
    passkey.labels().store("personal".to_string()).await?;
    // ANCHOR_END: store-label
    Ok(())
}

async fn single_cta_onboarding() -> Result<Wallet> {
    // ANCHOR: signin-fallback-register
    // Single-CTA onboarding: try silent sign_in first, fall through to
    // register on CredentialNotFound. The OS shows ONE prompt for a
    // returning user (silent assertion succeeds), TWO for a new user
    // (silent assertion fast-fails, then create + dual-salt assert).
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // Discovery mode (label = None): derives master + DEFAULT label
    // in a single ceremony. The fresh-device user fast-fails in <300ms
    // with no UI shown.
    match passkey
        .sign_in(SignInRequest {
            label: None,
            ..Default::default()
        })
        .await
    {
        Ok(response) => Ok(response.wallet),
        // CredentialNotFound is the SDK's classification for "no matching
        // credential on this device", including iOS's <300ms fast-fail
        // case where the platform conflates no-cred with user-cancel.
        Err(PasskeyError::Prf(PrfProviderError::CredentialNotFound(_))) => {
            // No credential. Onboard a new user.
            let response = passkey
                .register(RegisterRequest {
                    label: Some("personal".to_string()),
                    exclude_credential_ids: vec![],
                })
                .await?;
            Ok(response.wallet)
        }
        Err(e) => Err(e.into()),
    }
    // ANCHOR_END: signin-fallback-register
}

async fn check_domain() -> Result<()> {
    // ANCHOR: domain-association
    // Verify Apple AASA / Android Asset Links / Web Related Origins
    // before the first WebAuthn ceremony. Diagnostic only: never blocks.
    let prf_provider = Arc::new(CustomPrfProvider);
    let result = prf_provider.check_domain_association().await?;

    match result {
        DomainAssociation::Associated => {
            // Safe to proceed.
        }
        DomainAssociation::NotAssociated { source, reason } => {
            // Configuration is wrong (entitlement missing, AASA stale,
            // assetlinks malformed). Surface a developer-facing error.
            eprintln!("Domain association failed (source={source}): {reason}");
            return Ok(());
        }
        DomainAssociation::Skipped { reason: _ } => {
            // Verification could not be performed (offline, endpoint
            // timeout, no public-suffix match). Proceed normally: this
            // is NOT a negative signal.
        }
    }
    // ANCHOR_END: domain-association
    Ok(())
}

async fn recover_from_already_exists() -> Result<Wallet> {
    // ANCHOR: recover-already-exists
    // The OS rejected register because the user's password manager
    // already holds a credential matching `exclude_credential_ids`.
    // Route the user to the sign-in path: the OS picker will surface
    // the existing credential and the SDK's identity cache will warm
    // up on the assertion.
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    match passkey
        .register(RegisterRequest {
            label: Some("personal".to_string()),
            exclude_credential_ids: vec![
                // app-persisted credential IDs from prior registrations
            ],
        })
        .await
    {
        Ok(response) => Ok(response.wallet),
        Err(PasskeyError::Prf(PrfProviderError::CredentialAlreadyExists(_))) => {
            // Flip to sign-in. The existing credential's PRF output is
            // the same wallet seed the host would have minted on register.
            let response = passkey
                .sign_in(SignInRequest {
                    label: Some("personal".to_string()),
                    ..Default::default()
                })
                .await?;
            Ok(response.wallet)
        }
        Err(e) => Err(e.into()),
    }
    // ANCHOR_END: recover-already-exists
}

async fn handle_timeout() -> Result<SignInResponse> {
    // ANCHOR: handle-timeout
    // The OS biometric inactivity timeout (~55s+) tore down the prompt
    // without user intent. Distinct from a real cancel: hosts may
    // surface a re-prompt UI without treating it as the user opting
    // out. The SDK fires PrfProviderError::UserTimedOut when assertion
    // or register elapsed time crosses 55_000 ms.
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    match passkey
        .sign_in(SignInRequest {
            label: Some("personal".to_string()),
            ..Default::default()
        })
        .await
    {
        Ok(response) => Ok(response),
        Err(PasskeyError::Prf(PrfProviderError::UserTimedOut)) => {
            // Show a sticky retry screen with timeout-specific copy.
            // Do NOT auto-retry without user input.
            println!("Sign-in timed out: show \"Try Again\" UI.");
            Err(PasskeyError::Prf(PrfProviderError::UserTimedOut).into())
        }
        Err(e) => Err(e.into()),
    }
    // ANCHOR_END: handle-timeout
}
