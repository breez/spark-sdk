use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use breez_sdk_spark::Seed;
use breez_sdk_spark::passkey::{NostrRelayConfig, Passkey, PasskeyPrfError, PasskeyPrfProvider};

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
    /// Optional wallet name for seed derivation. If omitted, the core uses the default name.
    pub wallet_name: Option<String>,
    /// Whether to list and select from wallet names published to Nostr.
    pub list_wallet_names: bool,
    /// Whether to publish the wallet name to Nostr.
    pub store_wallet_name: bool,
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
    ) -> Result<Arc<dyn PasskeyPrfProvider>, PasskeyPrfError> {
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
    provider: Arc<dyn PasskeyPrfProvider>,
    breez_api_key: Option<String>,
    wallet_name: Option<String>,
    list_wallet_names: bool,
    store_wallet_name: bool,
) -> Result<Seed> {
    let relay_config = NostrRelayConfig {
        breez_api_key,
        ..NostrRelayConfig::default()
    };
    let passkey = Passkey::new(provider, Some(relay_config));

    // --store-wallet-name: publish the wallet name to Nostr and exit
    if store_wallet_name && let Some(wallet_name) = &wallet_name {
        println!("Publishing wallet name '{wallet_name}' to Nostr...");
        passkey
            .store_wallet_name(wallet_name.clone())
            .await
            .map_err(|e| anyhow!("Failed to store wallet name: {e}"))?;
        println!("Wallet name '{wallet_name}' published successfully.");
    }

    // --list-wallet-names: query Nostr and prompt user to select
    let wallet_name = if list_wallet_names {
        println!("Querying Nostr for available wallet names...");
        let wallet_names = passkey
            .list_wallet_names()
            .await
            .map_err(|e| anyhow!("Failed to list wallet names: {e}"))?;

        if wallet_names.is_empty() {
            return Err(anyhow!("No wallet names found on Nostr for this identity"));
        }

        println!("Available wallet names:");
        for (i, name) in wallet_names.iter().enumerate() {
            println!("  {}: {}", i + 1, name);
        }

        print!("Select wallet name (1-{}): ", wallet_names.len());
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let idx: usize = input
            .trim()
            .parse()
            .map_err(|_| anyhow!("Invalid selection"))?;

        if idx < 1 || idx > wallet_names.len() {
            return Err(anyhow!("Selection out of range"));
        }

        Some(wallet_names[idx - 1].clone())
    } else {
        wallet_name
    };

    let wallet = passkey
        .get_wallet(wallet_name)
        .await
        .map_err(|e| anyhow!("Failed to derive wallet: {e}"))?;
    Ok(wallet.seed)
}
