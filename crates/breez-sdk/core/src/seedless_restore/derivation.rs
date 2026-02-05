use super::error::SeedlessRestoreError;
use base64::{Engine, engine::general_purpose::STANDARD};
use bitcoin::{
    Network,
    bip32::{DerivationPath, Xpriv},
    hashes::{Hash, sha256},
    secp256k1::Secp256k1,
};

/// The magic salt for deriving the account master.
///
/// This is the hex-encoded ASCII string "NYOASTRTSAOYN", used as a domain separator
/// to derive the Nostr identity keypair. The account master is produced by
/// `PRF(passkey, ACCOUNT_MASTER_SALT)` and then used for BIP32 derivation of the
/// Nostr signing key at [`NOSTR_SALT_DERIVATION_PATH`].
pub const ACCOUNT_MASTER_SALT: &str = "4e594f415354525453414f594e";

/// Nostr derivation path for salt storage identity.
/// Uses account 55 which is dedicated to salt storage per the seedless-restore spec.
const NOSTR_SALT_DERIVATION_PATH: &str = "m/44'/1237'/55'/0/0";

/// Derives the Nostr keypair for salt storage from the account master.
///
/// The account master (32 bytes from PRF) is used as the seed for BIP32 derivation.
/// The Nostr key is derived at path `m/44'/1237'/55'/0/0` (account 55).
///
/// # Arguments
/// * `account_master` - The 32-byte PRF output from the magic salt
///
/// # Returns
/// * `Ok(nostr::Keys)` - The Nostr keypair for signing salt events
/// * `Err(SeedlessRestoreError)` - If derivation fails
pub fn derive_nostr_keypair(account_master: &[u8]) -> Result<nostr::Keys, SeedlessRestoreError> {
    if account_master.len() != 32 {
        return Err(SeedlessRestoreError::InvalidPrfOutput(format!(
            "Account master must be 32 bytes, got {}",
            account_master.len()
        )));
    }

    let secp = Secp256k1::new();

    // Use account_master as seed for BIP32 master key
    let master = Xpriv::new_master(Network::Bitcoin, account_master)?;

    // Derive at Nostr salt path: m/44'/1237'/55'/0/0
    let path: DerivationPath = NOSTR_SALT_DERIVATION_PATH.parse().map_err(|e| {
        SeedlessRestoreError::KeyDerivationError(format!("Invalid derivation path: {e}"))
    })?;

    let derived = master.derive_priv(&secp, &path)?;

    // Convert to nostr secret key
    let secret_key = nostr::SecretKey::from_slice(&derived.private_key.secret_bytes())
        .map_err(|e| SeedlessRestoreError::KeyDerivationError(e.to_string()))?;

    Ok(nostr::Keys::new(secret_key))
}

/// Derives a Nostr keypair for NIP-42 authentication from a Breez API key.
///
/// The derivation is: `sha256(base64_decode(api_key))` → 32-byte secret key → Nostr keypair.
/// This keypair is used to authenticate with the Breez relay via NIP-42.
///
/// # Arguments
/// * `api_key` - The Breez API key (base64 encoded)
///
/// # Returns
/// * `Ok(nostr::Keys)` - The Nostr keypair for NIP-42 authentication
/// * `Err(SeedlessRestoreError)` - If the API key is invalid base64 or derivation fails
pub fn derive_nip42_keypair(api_key: &str) -> Result<nostr::Keys, SeedlessRestoreError> {
    // 1. Base64 decode the API key
    let decoded = STANDARD.decode(api_key).map_err(|e| {
        SeedlessRestoreError::KeyDerivationError(format!("Invalid base64 API key: {e}"))
    })?;

    // 2. SHA256 hash to get 32-byte secret key
    let hash = sha256::Hash::hash(&decoded);

    // 3. Create Nostr keypair from hash
    let secret_key = nostr::SecretKey::from_slice(hash.as_byte_array())
        .map_err(|e| SeedlessRestoreError::KeyDerivationError(e.to_string()))?;

    Ok(nostr::Keys::new(secret_key))
}

/// Converts PRF output to a BIP39 mnemonic.
///
/// The PRF output (32 bytes) is used directly as entropy for a 24-word mnemonic.
///
/// # Arguments
/// * `prf_output` - The 32-byte PRF output from the wallet salt
///
/// # Returns
/// * `Ok(String)` - The 24-word BIP39 mnemonic
/// * `Err(SeedlessRestoreError)` - If conversion fails
pub fn prf_to_mnemonic(prf_output: &[u8]) -> Result<String, SeedlessRestoreError> {
    if prf_output.len() != 32 {
        return Err(SeedlessRestoreError::InvalidPrfOutput(format!(
            "PRF output must be 32 bytes for 24-word mnemonic, got {}",
            prf_output.len()
        )));
    }

    let mnemonic = bip39::Mnemonic::from_entropy(prf_output)?;
    Ok(mnemonic.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_nostr_keypair() {
        // Use a deterministic 32-byte account master
        let account_master = [0u8; 32];

        let keys = derive_nostr_keypair(&account_master).expect("Should derive keypair");

        // Verify we get a valid public key
        let pubkey = keys.public_key();
        assert!(!pubkey.to_string().is_empty());
    }

    #[test]
    fn test_derive_nostr_keypair_invalid_length() {
        let short_master = [0u8; 16];
        let result = derive_nostr_keypair(&short_master);
        assert!(result.is_err());
    }

    #[test]
    fn test_prf_to_mnemonic() {
        // Use deterministic 32-byte PRF output
        let prf_output = [0u8; 32];

        let mnemonic = prf_to_mnemonic(&prf_output).expect("Should create mnemonic");

        // Verify it's a 24-word mnemonic
        let words: Vec<&str> = mnemonic.split_whitespace().collect();
        assert_eq!(words.len(), 24);
    }

    #[test]
    fn test_prf_to_mnemonic_invalid_length() {
        let short_output = [0u8; 16];
        let result = prf_to_mnemonic(&short_output);
        assert!(result.is_err());
    }

    #[test]
    fn test_deterministic_derivation() {
        // Same input should always produce same output
        let account_master = [42u8; 32];

        let keys1 = derive_nostr_keypair(&account_master).unwrap();
        let keys2 = derive_nostr_keypair(&account_master).unwrap();

        assert_eq!(keys1.public_key(), keys2.public_key());
    }

    #[test]
    fn test_deterministic_mnemonic() {
        // Same input should always produce same mnemonic
        let prf_output = [42u8; 32];

        let mnemonic1 = prf_to_mnemonic(&prf_output).unwrap();
        let mnemonic2 = prf_to_mnemonic(&prf_output).unwrap();

        assert_eq!(mnemonic1, mnemonic2);
    }

    #[test]
    fn test_derive_nip42_keypair() {
        // Use a sample base64-encoded API key
        let api_key = base64::engine::general_purpose::STANDARD.encode(b"test-api-key");

        let keys = derive_nip42_keypair(&api_key).expect("Should derive keypair");

        // Verify we get a valid public key
        let pubkey = keys.public_key();
        assert!(!pubkey.to_string().is_empty());
    }

    #[test]
    fn test_derive_nip42_keypair_deterministic() {
        // Same API key should always produce same keypair
        let api_key = base64::engine::general_purpose::STANDARD.encode(b"test-api-key");

        let keys1 = derive_nip42_keypair(&api_key).unwrap();
        let keys2 = derive_nip42_keypair(&api_key).unwrap();

        assert_eq!(keys1.public_key(), keys2.public_key());
    }

    #[test]
    fn test_derive_nip42_keypair_different_keys() {
        // Different API keys should produce different keypairs
        let api_key1 = base64::engine::general_purpose::STANDARD.encode(b"api-key-1");
        let api_key2 = base64::engine::general_purpose::STANDARD.encode(b"api-key-2");

        let keys1 = derive_nip42_keypair(&api_key1).unwrap();
        let keys2 = derive_nip42_keypair(&api_key2).unwrap();

        assert_ne!(keys1.public_key(), keys2.public_key());
    }

    #[test]
    fn test_derive_nip42_keypair_invalid_base64() {
        let invalid_api_key = "not-valid-base64!!!";
        let result = derive_nip42_keypair(invalid_api_key);
        assert!(result.is_err());
    }
}
