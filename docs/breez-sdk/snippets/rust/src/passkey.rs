use anyhow::Result;
use breez_sdk_spark::passkey::{
    ConnectWithPasskeyRequest, DeriveSeedsOutput, DeriveSeedsRequest, DomainAssociation, ErrorKind,
    PasskeyAvailability, PasskeyClient, PasskeyCredential, PrfProvider, PrfProviderError,
    RegisterRequest, SignInRequest, SignInResponse, Wallet,
};
use breez_sdk_spark::{connect, default_config, ConnectRequest, Network};
use std::sync::Arc;

// ANCHOR: implement-prf-provider
/// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
/// file-backed). Only derive_seeds and is_supported are required.
struct CustomPrfProvider;

#[async_trait::async_trait]
impl PrfProvider for CustomPrfProvider {
    async fn derive_seeds(
        &self,
        _request: DeriveSeedsRequest,
    ) -> Result<DeriveSeedsOutput, PrfProviderError> {
        // Return one 32-byte PRF output per salt, in input order.
        todo!("Implement using WebAuthn or native passkey APIs")
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        todo!("Check platform passkey availability")
    }

    async fn create_passkey(
        &self,
        _exclude_credentials: Vec<Vec<u8>>,
    ) -> Result<PasskeyCredential, PrfProviderError> {
        // Register a credential and return its ID plus attestation.
        todo!("Implement registration via WebAuthn create() / native API")
    }

    async fn check_domain_association(&self) -> Result<DomainAssociation, PrfProviderError> {
        // Custom providers without a verification source return Skipped.
        Ok(DomainAssociation::Skipped {
            reason: "CustomPrfProvider does not verify domain association".to_string(),
        })
    }
}
// ANCHOR_END: implement-prf-provider

async fn check_availability() -> Result<()> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // ANCHOR: check-availability
    match passkey.check_availability().await? {
        PasskeyAvailability::Available => {
            // Passkey supported: proceed with connect_with_passkey. On web,
            // call PasskeyClient::supports_immediate_mediation to pick
            // single- vs two-button onboarding (native is always single).
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

fn setup_passkey_client() -> PasskeyClient {
    // ANCHOR: setup-client
    let prf_provider = Arc::new(CustomPrfProvider);
    PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None)
    // ANCHOR_END: setup-client
}

async fn connect_with_passkey_unified() -> Result<breez_sdk_spark::BreezSdk> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // ANCHOR: connect-with-passkey
    // Single-CTA onboarding: silent sign-in, fall through to register.
    // Without a label, a returning user's wallets are discovered in
    // `response.labels` (the response wallet is the default); a new user
    // gets a freshly registered default wallet.
    let response = passkey
        .connect_with_passkey(ConnectWithPasskeyRequest::default())
        .await?;
    if response.labels.len() > 1 {
        // Multiple wallets: let the user pick, then sign_in to the chosen label.
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

async fn sign_in_existing_user() -> Result<SignInResponse> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None);

    // ANCHOR: sign-in
    // Returning-user-only sign-in. No fall-through to register.
    Ok(passkey
        .sign_in(SignInRequest {
            label: Some("personal".to_string()),
            ..Default::default()
        })
        .await?)
    // ANCHOR_END: sign-in
}

async fn register_new_passkey() -> Result<breez_sdk_spark::BreezSdk> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // ANCHOR: register-passkey
    let response = passkey
        .register(RegisterRequest {
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
    // ANCHOR_END: register-passkey
    Ok(sdk)
}

async fn credential_metadata() -> Result<()> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // ANCHOR: credential-metadata
    let response = passkey
        .register(RegisterRequest {
            label: Some("personal".to_string()),
            ..Default::default()
        })
        .await?;

    if let Some(credential) = &response.credential {
        // Persist to reopen the same wallet on sign-in
        println!("{:?}", credential.credential_id);
        // Authenticator model (display hint, unverified)
        println!("{:?}", credential.aaguid);
        // Whether the passkey syncs across devices
        println!("{:?}", credential.backup_eligible);
    }

    // Pin the stored credential ID so the OS can't substitute a sibling,
    // which would derive a different wallet.
    let sign_in_response = passkey
        .sign_in(SignInRequest {
            label: Some("personal".to_string()),
            allow_credentials: Some(vec![/* stored credential_id bytes */]),
            ..Default::default()
        })
        .await?;
    // Pass to connect() to open the wallet
    println!("{:?}", sign_in_response.wallet.seed);
    // Label this wallet was derived from
    println!("{}", sign_in_response.wallet.label);
    // This passkey's labels (populated on discovery sign-in)
    println!("{:?}", sign_in_response.labels);
    // Credential signed in with (credential_id only)
    println!("{:?}", sign_in_response.credential);
    // ANCHOR_END: credential-metadata
    Ok(())
}

async fn list_labels() -> Result<Vec<String>> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None);
    // ANCHOR: list-labels
    let labels = passkey.labels().list().await?;
    for label in &labels {
        println!("Found label: {label}");
    }
    // ANCHOR_END: list-labels
    Ok(labels)
}

async fn store_label() -> Result<()> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, Some("<breez api key>".to_string()), None);
    // ANCHOR: store-label
    passkey.labels().store("personal".to_string()).await?;
    // ANCHOR_END: store-label
    Ok(())
}

async fn check_domain() -> Result<()> {
    // ANCHOR: domain-association
    // Diagnostic only: never blocks the ceremony.
    let prf_provider = Arc::new(CustomPrfProvider);
    let result = prf_provider.check_domain_association().await?;

    match result {
        DomainAssociation::Associated => {
            // Safe to proceed.
        }
        DomainAssociation::NotAssociated { source, reason } => {
            // Misconfigured (entitlement, AASA, or assetlinks). Surface a dev error.
            eprintln!("Domain association failed (source={source}): {reason}");
            return Ok(());
        }
        DomainAssociation::Skipped { reason: _ } => {
            // Could not verify (offline, no public-suffix match). Not a failure.
        }
    }
    // ANCHOR_END: domain-association
    Ok(())
}

async fn recover_from_already_exists() -> Result<Wallet> {
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // ANCHOR: recover-already-exists
    match passkey
        .register(RegisterRequest {
            label: Some("personal".to_string()),
            exclude_credentials: Some(vec![
                // app-persisted credential IDs from prior registrations
            ]),
        })
        .await
    {
        Ok(response) => Ok(response.wallet),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            // A matching credential already exists; sign in to it instead.
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
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None, None);

    // ANCHOR: handle-timeout
    // Biometric inactivity timeout, distinct from a user cancel.
    match passkey
        .sign_in(SignInRequest {
            label: Some("personal".to_string()),
            ..Default::default()
        })
        .await
    {
        Ok(response) => Ok(response),
        Err(e) if e.kind() == ErrorKind::Timeout => {
            // Show a retry UI. Do NOT auto-retry without user input.
            println!("Sign-in timed out: show \"Try Again\" UI.");
            Err(e.into())
        }
        Err(e) => Err(e.into()),
    }
    // ANCHOR_END: handle-timeout
}
