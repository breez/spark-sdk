use std::{path::PathBuf, str::FromStr};

use bitcoin::hashes::{Hash, sha256};

use crate::{Network, SdkError, Seed};

pub fn default_storage_path(
    data_dir: &str,
    network: &Network,
    seed: &Seed,
) -> Result<PathBuf, SdkError> {
    let storage_dir = std::path::PathBuf::from_str(data_dir)?;
    let path_suffix: String = match seed {
        crate::Seed::Mnemonic {
            mnemonic,
            passphrase,
        } => {
            // Ensure mnemonic is valid before proceeding
            bip39::Mnemonic::parse(mnemonic)
                .map_err(|e| SdkError::InvalidInput(format!("Invalid mnemonic: {e}")))?;
            let str = format!("{mnemonic}:{passphrase:?}");
            sha256::Hash::hash(str.as_bytes())
                .to_string()
                .chars()
                .take(8)
                .collect()
        }
        crate::Seed::Entropy(vec) => sha256::Hash::hash(vec.as_slice())
            .to_string()
            .chars()
            .take(8)
            .collect(),
    };

    Ok(storage_dir
        .join(network.to_string().to_lowercase())
        .join(path_suffix))
}
