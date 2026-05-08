use anyhow::Result;
use breez_sdk_spark::passkey::{
    CreatePasskeyRequest, DomainAssociation, NostrRelayConfig, PasskeyClient, PrfProvider,
    PrfProviderError, RegisterRequest, RegisteredCredential, SignInRequest,
};
use breez_sdk_spark::{ConnectRequest, Network, connect, default_config};
use std::sync::Arc;

// ANCHOR: implement-prf-provider
/// Implement the PrfProvider trait for custom logic if the built-in
/// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
/// air-gapped backup file, etc.). Single API surface: derive_seeds for
/// derivation, create_passkey for registration, is_supported and
/// check_domain_association for diagnostics. Single-salt derivation is
/// the trivial 1-element bulk case.
struct CustomPrfProvider;

#[async_trait::async_trait]
impl PrfProvider for CustomPrfProvider {
    async fn derive_seeds(
        &self,
        _salts: Vec<String>,
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
        _request: CreatePasskeyRequest,
    ) -> Result<RegisteredCredential, PrfProviderError> {
        // Register a new credential and return its ID + AAGUID + BE flag.
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

    if prf_provider.is_supported().await? {
        // Show passkey as primary option
    } else {
        // Fall back to mnemonic flow
    }
    // ANCHOR_END: check-availability
    Ok(())
}

async fn connect_with_passkey() -> Result<breez_sdk_spark::BreezSdk> {
    // ANCHOR: connect-with-passkey
    let prf_provider = Arc::new(CustomPrfProvider);
    let passkey = PasskeyClient::new(prf_provider, None);

    // sign_in derives the wallet seed for an existing credential. With
    // bulk PRF on iOS+Android this is a single OS prompt that derives
    // master + label seeds in one ceremony.
    let response = passkey
        .sign_in(SignInRequest {
            label: Some("personal".to_string()),
            extra_salts: vec![],
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
    let passkey = PasskeyClient::new(prf_provider, None);

    let response = passkey
        .register(RegisterRequest {
            label: Some("personal".to_string()),
            extra_salts: vec![],
            exclude_credential_ids: vec![],
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

async fn list_labels() -> Result<Vec<String>> {
    // ANCHOR: list-labels
    let prf_provider = Arc::new(CustomPrfProvider);
    let relay_config = NostrRelayConfig {
        breez_api_key: Some("<breez api key>".to_string()),
        ..NostrRelayConfig::default()
    };
    let passkey = PasskeyClient::new(prf_provider, Some(relay_config));

    // sign_in with no label runs in discovery mode: it derives the
    // master seed AND lists labels in the same ceremony, so a follow-up
    // list_labels() reads from the cached identity for free.
    let labels = passkey.list_labels().await?;

    for label in &labels {
        println!("Found label: {label}");
    }
    // ANCHOR_END: list-labels
    Ok(labels)
}

async fn store_label() -> Result<()> {
    // ANCHOR: store-label
    let prf_provider = Arc::new(CustomPrfProvider);
    let relay_config = NostrRelayConfig {
        breez_api_key: Some("<breez api key>".to_string()),
        ..NostrRelayConfig::default()
    };
    let passkey = PasskeyClient::new(prf_provider, Some(relay_config));

    // For a new label on an existing identity, call sign_in(new_label)
    // first to seed the SDK's identity cache via setup_wallet, THEN
    // store_label uses the cached identity for free (1 OS prompt total).
    passkey.store_label("personal".to_string()).await?;
    // ANCHOR_END: store-label
    Ok(())
}
