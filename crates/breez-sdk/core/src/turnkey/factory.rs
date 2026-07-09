//! Factory for the Turnkey-backed signers, exposed over uniffi.

use std::str::FromStr;
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use platform_utils::create_http_client;
use spark_wallet::SparkAddress;

use crate::error::SignerError;
use crate::signer::{ExternalBreezSigner, ExternalSigningSigner, ExternalSparkSigner};
use crate::{ExternalSigners, Network, SigningOnlyExternalSigners};

use super::accounts::spark_address_format;
use super::breez_signer::{TurnkeyBreezSigner, TurnkeySigningSigner};
use super::config::TurnkeyConfig;
use super::spark_signer::TurnkeySparkSigner;
use super::transport::TurnkeyClient;
use super::types::ADDRESS_FORMAT_COMPRESSED;

fn to_signer_err<E: std::fmt::Display>(e: E) -> SignerError {
    SignerError::Generic(e.to_string())
}

/// The Spark account number from `config`: explicit, or the per-network default
/// shared with the seed-based signer.
pub(crate) fn account_number(config: &TurnkeyConfig) -> u32 {
    config
        .account_number
        .unwrap_or_else(|| spark_wallet::default_account_number(config.network.into()))
}

/// Spark identity path; the Spark signer signs with this key inside the enclave.
fn identity_path(account: u32) -> String {
    format!("m/8797555'/{account}'/0'")
}

/// Sets up the shared Turnkey client and, on first-time init, materializes the
/// wallet's two identity accounts at the identity path: the compressed one
/// (ECDSA signWith for operator auth and messages) and the Spark-format one
/// (Schnorr/FROST signWith for Spark-protocol signing). The keys stay in the
/// enclave; `create_account` is idempotent, so repeating the unprovisioned path
/// is safe.
///
/// A supplied `config.identity_public_key` skips both round-trips. It assumes a
/// prior unprovisioned init for this wallet already materialized both accounts:
/// a seeded [`TurnkeySparkSigner`] serves the Spark address from memory and never
/// creates the account, so Spark signing fails if the Spark-format account was
/// never materialized (a key carried from a wallet first initialized by an SDK
/// version that created it lazily, before it ever signed). Omit the key to
/// re-materialize both.
async fn build_client(
    config: &TurnkeyConfig,
) -> Result<(Arc<TurnkeyClient>, Network, u32), SignerError> {
    let network = config.network;
    let account = account_number(config);
    let http = create_http_client(Some("breez-sdk-spark-turnkey"));
    let client = Arc::new(TurnkeyClient::new(config, http).map_err(to_signer_err)?);
    if config.identity_public_key.is_none() {
        client
            .create_account(identity_path(account), ADDRESS_FORMAT_COMPRESSED)
            .await
            .map_err(to_signer_err)?;
        client
            .create_account(identity_path(account), spark_address_format(network))
            .await
            .map_err(to_signer_err)?;
    }
    Ok((client, network, account))
}

/// The Spark signer for `config`, with its identity pubkey and Spark address
/// pre-seeded when `config.identity_public_key` is set, so init makes no Turnkey
/// call for them. The Spark address is the canonical address for the identity
/// key, derived locally rather than carried separately. An unset key falls back
/// to lazy fetching; a set but unparsable key is an error.
fn build_spark_signer(
    client: Arc<TurnkeyClient>,
    config: &TurnkeyConfig,
    network: Network,
    account: u32,
) -> Result<TurnkeySparkSigner, SignerError> {
    let Some(hex) = &config.identity_public_key else {
        return Ok(TurnkeySparkSigner::new(client, network, account));
    };
    let identity = PublicKey::from_str(hex).map_err(to_signer_err)?;
    let spark_address = SparkAddress::new(identity, network.into(), None)
        .to_address_string()
        .map_err(to_signer_err)?;
    Ok(TurnkeySparkSigner::new_seeded(
        client,
        network,
        account,
        Some(identity),
        Some(spark_address),
    ))
}

/// Builds the Turnkey-backed Breez and Spark signers from `config`, sharing one
/// Turnkey client.
///
/// The Spark signer keeps every signing operation in the Turnkey enclave; the
/// Breez signer does too, except ECIES and HMAC, which run locally against a
/// dedicated, non-Spark key exported on first use (see `TurnkeyBreezSigner`).
/// Exporting a non-Spark key keeps every Spark key (the identity key included)
/// in the enclave; ECIES/HMAC only need a stable key, not a Spark one.
///
/// For a wallet under a deny-export policy, use
/// [`create_turnkey_signing_only_signer`] instead: it never exports a key.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn create_turnkey_signer(config: TurnkeyConfig) -> Result<ExternalSigners, SignerError> {
    let (client, network, account) = build_client(&config).await?;
    let breez_signer: Arc<dyn ExternalBreezSigner> =
        Arc::new(TurnkeyBreezSigner::new(client.clone(), network, account));
    let spark_signer: Arc<dyn ExternalSparkSigner> =
        Arc::new(build_spark_signer(client, &config, network, account)?);
    Ok(ExternalSigners {
        breez_signer,
        spark_signer,
    })
}

/// Builds signing-only Turnkey-backed signers from `config`, for a wallet under
/// a deny-export policy. The Breez half performs signing only and never exports
/// a key, so no ECIES/HMAC is attempted. Pair with
/// [`connect_with_signing_only_signer`](crate::connect_with_signing_only_signer).
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn create_turnkey_signing_only_signer(
    config: TurnkeyConfig,
) -> Result<SigningOnlyExternalSigners, SignerError> {
    let (client, network, account) = build_client(&config).await?;
    let breez_signer: Arc<dyn ExternalSigningSigner> =
        Arc::new(TurnkeySigningSigner::new(client.clone(), network, account));
    let spark_signer: Arc<dyn ExternalSparkSigner> =
        Arc::new(build_spark_signer(client, &config, network, account)?);
    Ok(SigningOnlyExternalSigners {
        breez_signer,
        spark_signer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;
    use crate::signer::external_types::PublicKeyBytes;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    // Unroutable base URL: the seeded path must not touch the network, so a real
    // request would be a bug (and hang).
    fn test_config(identity_public_key: Option<String>) -> TurnkeyConfig {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let pk = sk.public_key(&secp);
        TurnkeyConfig {
            base_url: Some("https://turnkey.invalid".to_string()),
            organization_id: "test-org".to_string(),
            api_public_key: hex::encode(pk.serialize()),
            api_private_key: hex::encode(sk.secret_bytes()),
            wallet_id: "test-wallet".to_string(),
            network: Network::Regtest,
            account_number: Some(0),
            identity_public_key,
            retry: None,
            max_rps: None,
        }
    }

    fn identity_pubkey() -> PublicKey {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x22; 32]).unwrap();
        sk.public_key(&secp)
    }

    // A config carrying the identity pubkey builds the signing-only signer with
    // no network (the base URL is unroutable), and the Spark signer serves that
    // identity key from the seed.
    #[tokio::test]
    async fn seeded_identity_builds_signing_only_signer_offline() {
        let identity = identity_pubkey();
        let signers = create_turnkey_signing_only_signer(test_config(Some(hex::encode(
            identity.serialize(),
        ))))
        .await
        .unwrap();

        let served = signers
            .spark_signer
            .get_identity_public_key()
            .await
            .unwrap();
        assert_eq!(served, PublicKeyBytes::from_public_key(&identity));
    }

    // Same for the full signer: the seeded identity is served offline.
    #[tokio::test]
    async fn seeded_identity_builds_full_signer_offline() {
        let identity = identity_pubkey();
        let signers = create_turnkey_signer(test_config(Some(hex::encode(identity.serialize()))))
            .await
            .unwrap();

        let served = signers
            .spark_signer
            .get_identity_public_key()
            .await
            .unwrap();
        assert_eq!(served, PublicKeyBytes::from_public_key(&identity));
    }

    // A malformed identity pubkey fails fast at build rather than yielding a
    // signer that would misbehave later.
    #[tokio::test]
    async fn invalid_identity_pubkey_is_rejected() {
        let result =
            create_turnkey_signing_only_signer(test_config(Some("not-a-key".to_string()))).await;
        assert!(result.is_err());
    }
}
