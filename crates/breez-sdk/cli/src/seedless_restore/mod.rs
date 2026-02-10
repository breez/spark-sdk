use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use breez_sdk_spark::Seed;
use breez_sdk_spark::seedless_restore::{PasskeyPrfError, PasskeyPrfProvider, SeedlessRestore};

#[cfg(feature = "fido2")]
pub mod fido2_prf;
pub mod file_prf;
pub mod yubikey_prf;

#[cfg(feature = "fido2")]
use fido2_prf::Fido2PrfProvider;
use file_prf::FilePrfProvider;
use yubikey_prf::YubiKeyPrfProvider;

/// Seedless restore provider type.
#[derive(Clone)]
pub enum SeedlessProvider {
    File,
    YubiKey,
    /// FIDO2/WebAuthn PRF using CTAP2 hmac-secret extension.
    /// Compatible with browser `WebAuthn` PRF for cross-platform seed derivation.
    #[cfg(feature = "fido2")]
    Fido2,
}

/// Configuration for seedless restore.
#[derive(Clone)]
pub struct SeedlessConfig {
    /// The PRF provider to use.
    pub provider: SeedlessProvider,
    /// Optional salt for seed derivation. If omitted, lists available salts from Nostr.
    pub salt: Option<String>,
    /// Optional relying party ID for FIDO2 provider (default: keys.breez.technology).
    pub rpid: Option<String>,
}

impl std::str::FromStr for SeedlessProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "file" => SeedlessProvider::File,
            "yubikey" => SeedlessProvider::YubiKey,
            #[cfg(feature = "fido2")]
            "fido2" => SeedlessProvider::Fido2,
            #[cfg(not(feature = "fido2"))]
            "fido2" => {
                return Err(
                    "fido2 provider requires the 'fido2' feature (cargo run --features fido2)"
                        .to_string(),
                );
            }
            _ => return Err(format!("invalid seedless provider '{s}'")),
        })
    }
}

impl SeedlessProvider {
    #[allow(unused_variables, clippy::needless_pass_by_value)]
    pub fn into_provider(
        self,
        data_dir: &PathBuf,
        fido2_rp_id: Option<String>,
    ) -> Result<Arc<dyn PasskeyPrfProvider>, PasskeyPrfError> {
        match self {
            SeedlessProvider::File => Ok(Arc::new(FilePrfProvider::new(data_dir)?)),
            SeedlessProvider::YubiKey => Ok(Arc::new(YubiKeyPrfProvider::new()?)),
            #[cfg(feature = "fido2")]
            SeedlessProvider::Fido2 => Ok(Arc::new(Fido2PrfProvider::new(fido2_rp_id))),
        }
    }
}

#[allow(clippy::arithmetic_side_effects)]
/// Resolve a wallet seed using the seedless restore flow.
///
/// Either creates a new seed with the given salt, or lists available salts
/// from Nostr and prompts the user to select one for restore.
pub async fn resolve_seedless_seed(
    provider: Arc<dyn PasskeyPrfProvider>,
    seedless_salt: Option<String>,
) -> Result<Seed> {
    let seedless = SeedlessRestore::new(provider, None);

    if let Some(salt) = seedless_salt {
        println!("Deriving seed from seedless secret...");
        seedless
            .create_seed(salt)
            .await
            .map_err(|e| anyhow!("Seedless restore failed: {e}"))
    } else {
        println!("Querying Nostr for available salts...");
        let salts = seedless
            .list_salts()
            .await
            .map_err(|e| anyhow!("Failed to list salts: {e}"))?;

        if salts.is_empty() {
            return Err(anyhow!("No salts found on Nostr for this identity"));
        }

        println!("Available salts:");
        for (i, salt) in salts.iter().enumerate() {
            println!("  {}: {}", i + 1, salt);
        }

        print!("Select salt (1-{}): ", salts.len());
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let idx: usize = input
            .trim()
            .parse()
            .map_err(|_| anyhow!("Invalid selection"))?;

        if idx < 1 || idx > salts.len() {
            return Err(anyhow!("Selection out of range"));
        }

        let selected_salt = &salts[idx - 1];
        println!("Restoring seed for '{selected_salt}'...");
        seedless
            .restore_seed(selected_salt.clone())
            .await
            .map_err(|e| anyhow!("Seedless restore failed: {e}"))
    }
}
