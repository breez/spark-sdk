use anyhow::Result;
use breez_sdk_spark::passkey::{
    ConnectWithPasskeyRequest, DeriveSeedsRequest, DomainAssociation, ErrorKind,
    PasskeyAvailability, PasskeyClient, PasskeyConfig, PrfProvider, PrfProviderError,
    RegisterRequest, RegisteredCredential, SignInRequest, SignInResponse, Wallet,
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

async fn connect_with_passkey_unified() -> Result<breez_sdk_spark::BreezSdk> {
    // ANCHOR: connect-with-passkey
    // Single-CTA onboarding: silent sign-in, fall through to register.
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    let response = passkey
        .connect_with_passkey(ConnectWithPasskeyRequest {
            label: Some("personal".to_string()),
            exclude_credential_ids: vec![],
        })
        .await?;

    // `registered_credential` is the path discriminator (None on sign-in).
    if let Some(credential) = &response.registered_credential {
        let _persist = credential.credential_id.clone();
    }

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
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    let response = passkey
        .register(RegisterRequest {
            label: Some("personal".to_string()),
            exclude_credential_ids: vec![],
        })
        .await?;

    // Persist credential_id for future exclude_credential_ids.
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
    // first to warm the SDK's identity cache, THEN
    // labels().store() uses the cached identity for free (1 OS prompt total).
    passkey.labels().store("personal".to_string()).await?;
    // ANCHOR_END: store-label
    Ok(())
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
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
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
    // Timeout is distinct from a cancel: surface a re-prompt UI.
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
        Err(e) if e.kind() == ErrorKind::Timeout => {
            // Show a sticky retry screen with timeout-specific copy.
            // Do NOT auto-retry without user input.
            println!("Sign-in timed out: show \"Try Again\" UI.");
            Err(e.into())
        }
        Err(e) => Err(e.into()),
    }
    // ANCHOR_END: handle-timeout
}
