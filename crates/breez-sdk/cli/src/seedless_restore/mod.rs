use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use breez_sdk_spark::Seed;
use breez_sdk_spark::seedless_restore::{PasskeyPrfError, PasskeyPrfProvider, SeedlessRestore};

pub mod file_prf;
pub mod yubikey_prf;

use file_prf::FilePrfProvider;
use yubikey_prf::YubiKeyPrfProvider;

#[derive(Clone)]
pub enum SeedlessProvider {
    File,
    YubiKey,
}

impl std::str::FromStr for SeedlessProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "file" => SeedlessProvider::File,
            "yubikey" => SeedlessProvider::YubiKey,
            _ => return Err(format!("invalid seedless provider '{s}'")),
        })
    }
}

impl SeedlessProvider {
    pub fn into_provider(
        self,
        data_dir: &PathBuf,
    ) -> Result<Arc<dyn PasskeyPrfProvider>, PasskeyPrfError> {
        match self {
            SeedlessProvider::File => Ok(Arc::new(FilePrfProvider::new(data_dir)?)),
            SeedlessProvider::YubiKey => Ok(Arc::new(YubiKeyPrfProvider::new()?)),
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
