use bitcoin::hashes::{Hash, sha256};
use breez_sdk_spark::seedless_restore::{PasskeyPrfError, PasskeyPrfProvider};
use ctap_hid_fido2::fidokey::get_assertion::get_assertion_params::{
    Extension as GetExtension, GetAssertionArgsBuilder,
};
use ctap_hid_fido2::fidokey::make_credential::make_credential_params::Extension as MakeExtension;
use ctap_hid_fido2::fidokey::make_credential::make_credential_params::MakeCredentialArgsBuilder;
use ctap_hid_fido2::public_key_credential_user_entity::PublicKeyCredentialUserEntity;
use ctap_hid_fido2::{Cfg, FidoKeyHidFactory};
use std::sync::{Arc, Mutex};

/// `WebAuthn` PRF salt prefix for domain separation.
/// Per W3C spec: actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
const WEBAUTHN_PRF_PREFIX: &[u8] = b"WebAuthn PRF";

/// Default relying party ID for passkeys.
pub const DEFAULT_RP_ID: &str = "keys.breez.technology";

/// Default relying party name.
const DEFAULT_RP_NAME: &str = "Breez SDK";

/// Create FIDO2 config with our custom touch prompts (suppress library's default message).
fn fido2_cfg() -> Cfg {
    Cfg {
        keep_alive_msg: String::new(), // We print our own touch prompts
        ..Cfg::init()
    }
}

/// Cached state shared across calls (PIN only - credential is discoverable).
#[derive(Default)]
struct CachedState {
    pin: Option<String>,
}

/// FIDO2 hmac-secret implementation of `PasskeyPrfProvider`.
///
/// Uses CTAP2 hmac-secret extension for browser-compatible PRF.
/// Applies `WebAuthn` salt transformation for cross-platform compatibility.
///
/// Uses discoverable credentials (resident keys) so no credential storage is needed.
/// The credential lives on the authenticator and is discovered by rpId.
pub struct Fido2PrfProvider {
    rp_id: String,
    rp_name: String,
    cache: Arc<Mutex<CachedState>>,
}

impl Fido2PrfProvider {
    /// Create a new `Fido2PrfProvider`.
    ///
    /// # Arguments
    /// * `rp_id` - The relying party ID (must match web app for cross-platform compatibility)
    pub fn new(rp_id: Option<String>) -> Self {
        Self {
            rp_id: rp_id.unwrap_or_else(|| DEFAULT_RP_ID.to_string()),
            rp_name: DEFAULT_RP_NAME.to_string(),
            cache: Arc::new(Mutex::new(CachedState::default())),
        }
    }

    /// Transform developer salt to `WebAuthn`-compatible salt.
    ///
    /// Implements: actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
    /// This matches what browsers do internally for the `WebAuthn` PRF extension.
    #[allow(clippy::arithmetic_side_effects)]
    fn transform_salt(salt: &[u8]) -> [u8; 32] {
        let mut input = Vec::with_capacity(WEBAUTHN_PRF_PREFIX.len() + 1 + salt.len());
        input.extend_from_slice(WEBAUTHN_PRF_PREFIX);
        input.push(0x00);
        input.extend_from_slice(salt);

        sha256::Hash::hash(&input).to_byte_array()
    }

    /// Get PIN from cache, environment variable, or interactive prompt.
    fn get_pin(cache: &Arc<Mutex<CachedState>>) -> Result<String, PasskeyPrfError> {
        // Check cached PIN first
        if let Some(pin) = cache.lock().unwrap().pin.as_ref() {
            return Ok(pin.clone());
        }

        // Check environment variable for non-interactive use
        if let Ok(pin) = std::env::var("FIDO2_PIN") {
            cache.lock().unwrap().pin = Some(pin.clone());
            return Ok(pin);
        }

        // Interactive prompt
        eprint!("Enter FIDO2 PIN: ");
        let pin = rpassword::read_password()
            .map_err(|e| PasskeyPrfError::Generic(format!("Failed to read PIN: {e}")))?;

        // Cache for session
        cache.lock().unwrap().pin = Some(pin.clone());

        Ok(pin)
    }

    /// Register a discoverable credential with hmac-secret extension enabled.
    fn register_discoverable_credential(
        rp_id: &str,
        rp_name: &str,
        pin: &str,
    ) -> Result<(), PasskeyPrfError> {
        eprintln!("Creating new passkey for {rp_id}. Touch your authenticator...");

        let device = FidoKeyHidFactory::create(&fido2_cfg())
            .map_err(|e| PasskeyPrfError::Generic(format!("No FIDO2 device found: {e}")))?;

        // Random challenge (not used for verification, just required by protocol)
        let challenge: [u8; 32] = rand::random();

        // Random user ID
        let user_id: [u8; 16] = rand::random();

        let user_entity =
            PublicKeyCredentialUserEntity::new(Some(&user_id), Some(rp_name), Some(rp_name));

        let args = MakeCredentialArgsBuilder::new(rp_id, &challenge)
            .pin(pin)
            .user_entity(&user_entity)
            .extensions(&[MakeExtension::HmacSecret(Some(true))])
            .resident_key()
            .build();

        let result = device
            .make_credential_with_args(&args)
            .map_err(|e| map_fido2_error(e, "Credential creation failed"))?;

        // Verify hmac-secret was enabled
        let hmac_enabled = result
            .extensions
            .iter()
            .any(|ext| matches!(ext, MakeExtension::HmacSecret(Some(true))));

        if !hmac_enabled {
            return Err(PasskeyPrfError::PrfNotSupported);
        }

        eprintln!("Passkey created successfully with PRF support.");
        Ok(())
    }

    /// Get assertion using discoverable credential with hmac-secret PRF.
    ///
    /// Uses `get_assertion_with_args` with empty credential list for discoverable + PRF
    /// in a single step (like browser `WebAuthn` with `allowCredentials: []`).
    fn get_assertion_with_prf(
        rp_id: &str,
        transformed_salt: [u8; 32],
        pin: &str,
    ) -> Result<Vec<u8>, PasskeyPrfError> {
        let device = FidoKeyHidFactory::create(&fido2_cfg())
            .map_err(|e| PasskeyPrfError::Generic(format!("No FIDO2 device found: {e}")))?;

        eprintln!("Touch your authenticator...");
        let challenge: [u8; 32] = rand::random();

        // Use get_assertion_with_args with empty credential list (discoverable)
        // and hmac-secret extension - single step like browser WebAuthn
        let args = GetAssertionArgsBuilder::new(rp_id, &challenge)
            .pin(pin)
            .extensions(&[GetExtension::HmacSecret(Some(transformed_salt))])
            .build();

        let assertions = device
            .get_assertion_with_args(&args)
            .map_err(|e| map_fido2_error(e, "PRF assertion failed"))?;

        let assertion = assertions
            .first()
            .ok_or(PasskeyPrfError::CredentialNotFound)?;

        // Extract hmac-secret output from extensions
        for ext in &assertion.extensions {
            if let GetExtension::HmacSecret(Some(hmac_output)) = ext {
                return Ok(hmac_output.to_vec());
            }
        }

        Err(PasskeyPrfError::PrfNotSupported)
    }
}

#[async_trait::async_trait]
impl PasskeyPrfProvider for Fido2PrfProvider {
    async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
        let rp_id = self.rp_id.clone();
        let rp_name = self.rp_name.clone();
        let cache = Arc::clone(&self.cache);

        // Transform salt for WebAuthn compatibility
        let transformed_salt = Self::transform_salt(salt.as_bytes());

        // Use spawn_blocking for HID operations
        tokio::task::spawn_blocking(move || {
            let pin = Self::get_pin(&cache)?;

            // Try to get assertion first (credential may already exist)
            match Self::get_assertion_with_prf(&rp_id, transformed_salt, &pin) {
                Ok(output) => Ok(output),
                Err(PasskeyPrfError::CredentialNotFound) => {
                    // No credential for this rpId - register one
                    eprintln!("No passkey found for {rp_id}.");
                    Self::register_discoverable_credential(&rp_id, &rp_name, &pin)?;
                    // Now try again
                    Self::get_assertion_with_prf(&rp_id, transformed_salt, &pin)
                }
                Err(e) => Err(e),
            }
        })
        .await
        .map_err(|e| PasskeyPrfError::Generic(format!("Task join error: {e}")))?
    }

    async fn is_prf_available(&self) -> Result<bool, PasskeyPrfError> {
        // Check if a FIDO2 device is connected
        match FidoKeyHidFactory::create(&fido2_cfg()) {
            Ok(device) => {
                // Check device supports hmac-secret
                let info = device
                    .get_info()
                    .map_err(|e| PasskeyPrfError::Generic(e.to_string()))?;
                Ok(info.extensions.iter().any(|ext| ext == "hmac-secret"))
            }
            Err(_) => Ok(false),
        }
    }
}

/// Map ctap-hid-fido2 errors to `PasskeyPrfError`.
fn map_fido2_error(e: impl std::fmt::Display, context: &str) -> PasskeyPrfError {
    let msg = format!("{e}");

    if msg.contains("NO_CREDENTIALS") || msg.contains("no credential") {
        PasskeyPrfError::CredentialNotFound
    } else if msg.contains("PIN") && (msg.contains("invalid") || msg.contains("Invalid")) {
        PasskeyPrfError::AuthenticationFailed("Invalid PIN".into())
    } else if msg.contains("PIN") && msg.contains("blocked") {
        PasskeyPrfError::AuthenticationFailed("PIN blocked - reset required".into())
    } else if msg.contains("cancel") || msg.contains("Cancel") {
        PasskeyPrfError::UserCancelled
    } else if msg.contains("hmac-secret") && msg.contains("not supported") {
        PasskeyPrfError::PrfNotSupported
    } else {
        PasskeyPrfError::PrfEvaluationFailed(format!("{context}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_salt_transformation() {
        // Test that our salt transformation matches the WebAuthn PRF spec
        // actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
        let salt = b"test-salt";
        let transformed = Fido2PrfProvider::transform_salt(salt);

        // Verify it's 32 bytes
        assert_eq!(transformed.len(), 32);

        // Manually compute expected value
        let mut expected_input = b"WebAuthn PRF".to_vec();
        expected_input.push(0x00);
        expected_input.extend_from_slice(salt);
        let expected = sha256::Hash::hash(&expected_input).to_byte_array();

        assert_eq!(transformed, expected);
    }

    #[test]
    fn test_different_salts_produce_different_outputs() {
        let salt1 = Fido2PrfProvider::transform_salt(b"wallet-1");
        let salt2 = Fido2PrfProvider::transform_salt(b"wallet-2");

        assert_ne!(salt1, salt2);
    }
}
