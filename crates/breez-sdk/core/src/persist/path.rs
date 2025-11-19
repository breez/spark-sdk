use std::{path::PathBuf, str::FromStr};

use bitcoin::hashes::{Hash, sha256};
use spark_wallet::PublicKey;

use crate::{Network, SdkError};

pub fn default_storage_path(
    data_dir: &str,
    network: &Network,
    identity_pub_key: &PublicKey,
) -> Result<PathBuf, SdkError> {
    let storage_dir = std::path::PathBuf::from_str(data_dir)?;
    let path_suffix = sha256::Hash::hash(&identity_pub_key.serialize())
        .to_string()
        .chars()
        .take(8)
        .collect::<String>();

    Ok(storage_dir
        .join(network.to_string().to_lowercase())
        .join(path_suffix))
}
