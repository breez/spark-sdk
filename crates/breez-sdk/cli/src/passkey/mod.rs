use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use breez_sdk_spark::Seed;
use breez_sdk_spark::passkey::{PasskeyClient, PrfProvider, PrfProviderError, SignInRequest};

#[cfg(feature = "fido2")]
pub mod fido2_prf;
pub mod file_prf;
pub mod yubikey_prf;

#[cfg(feature = "fido2")]
use fido2_prf::Fido2PrfProvider;
use file_prf::FilePrfProvider;
use yubikey_prf::YubiKeyPrfProvider;

/// Passkey PRF provider type.
#[derive(Clone)]
pub enum PasskeyProvider {
    File,
    YubiKey,
    /// FIDO2/WebAuthn PRF using CTAP2 hmac-secret extension.
    /// Compatible with browser `WebAuthn` PRF for cross-platform seed derivation.
    #[cfg(feature = "fido2")]
    Fido2,
}

/// Configuration for passkey seed derivation.
#[derive(Clone)]
pub struct PasskeyConfig {
    /// The PRF provider to use.
    pub provider: PasskeyProvider,
    /// Optional label for seed derivation. If omitted, the core uses the default label.
    pub label: Option<String>,
    /// Whether to list and select from labels published to Nostr.
    pub list_labels: bool,
    /// Whether to publish the label to Nostr.
    pub store_label: bool,
    /// Optional relying party ID for FIDO2 provider (default: keys.breez.technology).
    pub rpid: Option<String>,
}

impl std::str::FromStr for PasskeyProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "file" => PasskeyProvider::File,
            "yubikey" => PasskeyProvider::YubiKey,
            #[cfg(feature = "fido2")]
            "fido2" => PasskeyProvider::Fido2,
            #[cfg(not(feature = "fido2"))]
            "fido2" => {
                return Err(
                    "fido2 provider requires the 'fido2' feature (cargo run --features fido2)"
                        .to_string(),
                );
            }
            _ => return Err(format!("invalid passkey provider '{s}'")),
        })
    }
}

impl PasskeyProvider {
    #[allow(unused_variables, clippy::needless_pass_by_value)]
    pub fn into_provider(
        self,
        data_dir: &PathBuf,
        fido2_rp_id: Option<String>,
    ) -> Result<Arc<dyn PrfProvider>, PrfProviderError> {
        match self {
            PasskeyProvider::File => Ok(Arc::new(FilePrfProvider::new(data_dir)?)),
            PasskeyProvider::YubiKey => Ok(Arc::new(YubiKeyPrfProvider::new()?)),
            #[cfg(feature = "fido2")]
            PasskeyProvider::Fido2 => Ok(Arc::new(Fido2PrfProvider::new(fido2_rp_id))),
        }
    }
}

#[allow(clippy::arithmetic_side_effects)]
pub async fn resolve_passkey_seed(
    provider: Arc<dyn PrfProvider>,
    breez_api_key: Option<String>,
    label: Option<String>,
    list_labels: bool,
    store_label: bool,
) -> Result<Seed> {
    let client = PasskeyClient::new(provider, breez_api_key, None);

    // --list-labels: discovery sign-in (no cached label) returns the
    // discovered label set; prompt user to pick.
    let label = if list_labels {
        println!("Querying Nostr for available labels...");
        let response = client
            .sign_in(SignInRequest {
                label: None,
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow!("Failed to discover labels: {e}"))?;

        if response.labels.is_empty() {
            return Err(anyhow!("No labels found on Nostr for this identity"));
        }

        println!("Available labels:");
        for (i, name) in response.labels.iter().enumerate() {
            println!("  {}: {}", i + 1, name);
        }

        print!("Select label (1-{}): ", response.labels.len());
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let idx: usize = input
            .trim()
            .parse()
            .map_err(|_| anyhow!("Invalid selection"))?;

        if idx < 1 || idx > response.labels.len() {
            return Err(anyhow!("Selection out of range"));
        }

        Some(response.labels[idx - 1].clone())
    } else {
        label
    };

    // --store-label: publish before signing in so a fresh client can
    // discover the label later.
    if store_label && let Some(label) = &label {
        println!("Publishing label '{label}' to Nostr...");
        client
            .labels()
            .store(label.clone())
            .await
            .map_err(|e| anyhow!("Failed to store label: {e}"))?;
        println!("Label '{label}' published successfully.");
    }

    let response = client
        .sign_in(SignInRequest {
            label,
            ..Default::default()
        })
        .await
        .map_err(|e| anyhow!("Failed to derive wallet: {e}"))?;
    Ok(response.wallet.seed)
}
