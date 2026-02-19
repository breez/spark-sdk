use crate::{
    CheckMessageRequest, CheckMessageResponse, ExternalInputParser, InputType, Logger, Network,
    SparkStatus, error::SdkError, models::Config,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use {
    crate::{ConnectRequest, models::KeySetConfig, sdk::BreezSdk, sdk_builder::SdkBuilder},
    std::sync::Arc,
};

/// Top-level namespace for the Breez SDK.
///
/// `Breez` groups all static/global SDK functions that don't require a wallet
/// connection. Use [`Breez::connect`] (non-WASM) or the existing [`connect`](crate::connect)
/// free function to obtain a [`BreezSdk`] (also exported as [`BreezClient`]) instance.
///
/// # Examples
///
/// ```rust,no_run
/// use breez_sdk_spark::{Breez, Network};
///
/// let config = Breez::default_config(Network::Mainnet);
/// ```
pub struct Breez;

#[allow(deprecated)]
impl Breez {
    /// Returns a default SDK configuration for the given network.
    ///
    /// This is equivalent to the [`default_config`](crate::default_config) free function.
    pub fn default_config(network: Network) -> Config {
        crate::default_config(network)
    }

    /// Parses a payment input string and returns the identified type.
    ///
    /// Supports BOLT11 invoices, Lightning addresses, LNURL variants, Bitcoin
    /// addresses, Spark addresses/invoices, BIP21 URIs, and more.
    ///
    /// This is equivalent to the [`parse_input`](crate::parse_input) free function.
    pub async fn parse(
        input: &str,
        external_input_parsers: Option<Vec<ExternalInputParser>>,
    ) -> Result<InputType, SdkError> {
        crate::parse_input(input, external_input_parsers).await
    }

    /// Initializes the SDK logging subsystem.
    ///
    /// This is equivalent to the [`init_logging`](crate::init_logging) free function.
    pub fn init_logging(
        log_dir: Option<String>,
        app_logger: Option<Box<dyn Logger>>,
        log_filter: Option<String>,
    ) -> Result<(), SdkError> {
        crate::init_logging(log_dir, app_logger, log_filter)
    }

    /// Fetches the current status of Spark network services.
    ///
    /// This is equivalent to the [`get_spark_status`](crate::get_spark_status) free function.
    pub async fn get_spark_status() -> Result<SparkStatus, SdkError> {
        crate::get_spark_status().await
    }

    /// Verifies a signed message against a public key.
    ///
    /// This is a pure cryptographic operation that does not require a wallet
    /// connection. The message is SHA256 hashed before verification.
    ///
    /// The signature can be hex-encoded in either DER or compact format.
    ///
    /// This is equivalent to the [`verify_message`](crate::verify_message) free function.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use breez_sdk_spark::{Breez, CheckMessageRequest};
    ///
    /// let result = Breez::verify_message(CheckMessageRequest {
    ///     message: "hello".to_string(),
    ///     pubkey: "<pubkey>".to_string(),
    ///     signature: "<signature>".to_string(),
    /// });
    /// ```
    pub fn verify_message(request: CheckMessageRequest) -> Result<CheckMessageResponse, SdkError> {
        crate::verify_message(request)
    }
}

// Non-WASM-only methods
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[allow(deprecated)]
impl Breez {
    /// Connects to the Spark network using the provided configuration and seed.
    ///
    /// This is equivalent to the [`connect`](crate::connect) free function.
    ///
    /// # Arguments
    ///
    /// * `request` - The connection request containing config, seed, and storage directory
    ///
    /// # Returns
    ///
    /// An initialized [`BreezSdk`] instance (also available as [`BreezClient`])
    pub async fn connect(request: ConnectRequest) -> Result<BreezSdk, SdkError> {
        crate::connect(request).await
    }

    /// Connects to the Spark network using an external signer.
    ///
    /// This is equivalent to the [`connect_with_signer`](crate::connect_with_signer) free function.
    pub async fn connect_with_signer(
        request: crate::ConnectWithSignerRequest,
    ) -> Result<BreezSdk, SdkError> {
        crate::connect_with_signer(request).await
    }

    /// Creates a default external signer from a mnemonic phrase.
    ///
    /// This is equivalent to the [`default_external_signer`](crate::default_external_signer) free function.
    pub fn default_external_signer(
        mnemonic: String,
        passphrase: Option<String>,
        network: Network,
        key_set_config: Option<KeySetConfig>,
    ) -> Result<Arc<dyn crate::signer::ExternalSigner>, SdkError> {
        crate::default_external_signer(mnemonic, passphrase, network, key_set_config)
    }

    /// Creates an SDK builder for advanced configuration.
    ///
    /// Use this when you need to customize storage, chain services, or other
    /// provider implementations.
    pub fn builder(config: Config, seed: crate::Seed) -> SdkBuilder {
        SdkBuilder::new(config, seed)
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn test_breez_default_config_matches_free_function() {
        let from_breez = Breez::default_config(Network::Mainnet);
        let from_free = crate::default_config(Network::Mainnet);

        assert!(
            matches!(from_breez.network, Network::Mainnet),
            "Expected Mainnet network"
        );
        assert_eq!(from_breez.api_key, from_free.api_key);
        assert_eq!(from_breez.sync_interval_secs, from_free.sync_interval_secs);
        assert_eq!(from_breez.lnurl_domain, from_free.lnurl_domain);
        assert_eq!(
            from_breez.prefer_spark_over_lightning,
            from_free.prefer_spark_over_lightning
        );
        assert_eq!(
            from_breez.private_enabled_default,
            from_free.private_enabled_default
        );
    }

    #[test]
    fn test_breez_default_config_regtest() {
        let config = Breez::default_config(Network::Regtest);

        assert!(
            matches!(config.network, Network::Regtest),
            "Expected Regtest network"
        );
        assert!(config.lnurl_domain.is_none());
    }

    #[test]
    fn test_breez_client_type_alias_compiles() {
        // Verify the type alias is usable at compile time
        fn _takes_breez_client(_sdk: &crate::BreezClient) {}
        fn _takes_breez_sdk(_sdk: &crate::BreezSdk) {}

        // Both should accept the same type — this is a compile-time test
    }

    #[test]
    fn test_verify_message_valid_signature() {
        use bitcoin::hashes::{Hash, sha256};
        use bitcoin::secp256k1::{Secp256k1, SecretKey};

        // Generate a keypair and sign a message
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0xcd; 32]).unwrap();
        let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

        let message = "hello world";
        let msg_hash = sha256::Hash::hash(message.as_bytes());
        let msg = bitcoin::secp256k1::Message::from_digest(msg_hash.to_byte_array());
        let signature = secp.sign_ecdsa(&msg, &secret_key);

        // Verify using Breez::verify_message()
        let result = Breez::verify_message(CheckMessageRequest {
            message: message.to_string(),
            pubkey: public_key.to_string(),
            signature: hex::encode(signature.serialize_der()),
        });
        assert!(result.is_ok());
        assert!(result.unwrap().is_valid);

        // Also verify with compact encoding
        let result_compact = Breez::verify_message(CheckMessageRequest {
            message: message.to_string(),
            pubkey: public_key.to_string(),
            signature: hex::encode(signature.serialize_compact()),
        });
        assert!(result_compact.is_ok());
        assert!(result_compact.unwrap().is_valid);
    }

    #[test]
    fn test_verify_message_invalid_signature() {
        use bitcoin::hashes::{Hash, sha256};
        use bitcoin::secp256k1::{Secp256k1, SecretKey};

        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0xcd; 32]).unwrap();
        let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

        let message = "hello world";
        let wrong_message = "wrong message";
        let msg_hash = sha256::Hash::hash(wrong_message.as_bytes());
        let msg = bitcoin::secp256k1::Message::from_digest(msg_hash.to_byte_array());
        let signature = secp.sign_ecdsa(&msg, &secret_key);

        // Verify with the wrong message — should return is_valid: false
        let result = Breez::verify_message(CheckMessageRequest {
            message: message.to_string(),
            pubkey: public_key.to_string(),
            signature: hex::encode(signature.serialize_der()),
        });
        assert!(result.is_ok());
        assert!(!result.unwrap().is_valid);
    }

    #[test]
    fn test_verify_message_invalid_pubkey() {
        let result = Breez::verify_message(CheckMessageRequest {
            message: "hello".to_string(),
            pubkey: "not-a-pubkey".to_string(),
            signature: "deadbeef".to_string(),
        });
        assert!(matches!(
            result,
            Err(crate::SdkError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_verify_message_invalid_signature_hex() {
        use bitcoin::secp256k1::{Secp256k1, SecretKey};

        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0xcd; 32]).unwrap();
        let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

        let result = Breez::verify_message(CheckMessageRequest {
            message: "hello".to_string(),
            pubkey: public_key.to_string(),
            signature: "not-hex".to_string(),
        });
        assert!(matches!(
            result,
            Err(crate::SdkError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_verify_message_matches_free_function() {
        use bitcoin::hashes::{Hash, sha256};
        use bitcoin::secp256k1::{Secp256k1, SecretKey};

        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0xab; 32]).unwrap();
        let public_key = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

        let message = "test message";
        let msg_hash = sha256::Hash::hash(message.as_bytes());
        let msg = bitcoin::secp256k1::Message::from_digest(msg_hash.to_byte_array());
        let signature = secp.sign_ecdsa(&msg, &secret_key);

        let pubkey_str = public_key.to_string();
        let sig_hex = hex::encode(signature.serialize_der());

        let from_breez = Breez::verify_message(CheckMessageRequest {
            message: message.to_string(),
            pubkey: pubkey_str.clone(),
            signature: sig_hex.clone(),
        })
        .unwrap();

        let from_free = crate::verify_message(CheckMessageRequest {
            message: message.to_string(),
            pubkey: pubkey_str,
            signature: sig_hex,
        })
        .unwrap();

        assert_eq!(from_breez.is_valid, from_free.is_valid);
        assert!(from_breez.is_valid);
    }
}
