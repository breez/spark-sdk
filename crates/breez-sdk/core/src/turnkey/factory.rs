//! Factory for the Turnkey-backed signers, exposed over uniffi.

use std::str::FromStr;
use std::sync::Arc;

use bitcoin::secp256k1::{PublicKey, SecretKey};
use platform_utils::create_http_client;
use serde::{Deserialize, Serialize};
use spark_wallet::SparkAddress;

use crate::ExternalSigners;
use crate::error::SignerError;
use crate::signer::breez::BreezSignerImpl;
use crate::signer::{ExternalBreezSigner, ExternalSparkSigner};

use super::accounts::{encryption_key_path, spark_address_format, xpriv_from_secret};
use super::breez_signer::{EncryptionBackend, TurnkeyBreezSigner};
use super::config::TurnkeyConfig;
use super::error::TurnkeyError;
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

/// Layout version of [`TurnkeyProvisionedSigner`]'s bytes. Bumped when the blob
/// contents change so an older blob is rejected as outdated rather than
/// misread; the caller re-provisions to upgrade.
const PROVISION_VERSION: u16 = 2;

/// The encryption-key verdict captured at provisioning time.
#[derive(Serialize, Deserialize)]
enum ProvisionedEncryption {
    /// Exportable key, already exported: seed the local ECIES/HMAC backend and
    /// never touch the network.
    Key([u8; 32]),
    /// Export is denied by wallet policy (a 403 at provisioning time): encryption
    /// is unavailable. Recorded so `create` never re-probes; re-provision after
    /// changing the policy to pick up a now-exportable key.
    Unavailable,
}

/// Versioned, persisted provisioning state. Bound to the wallet/network/account
/// so a blob paired with the wrong config (or an older layout) is rejected.
///
/// Alongside the encryption verdict it carries the wallet's stable Spark values,
/// the identity public key (hex) and Spark address, so a provisioned per-request
/// signer serves them from memory instead of re-fetching them from Turnkey on
/// every request. Both are fixed per wallet. Dynamic per-leaf keys are not here:
/// they change with the wallet's leaves and stay lazy.
#[derive(Serialize, Deserialize)]
struct ProvisionBlob {
    version: u16,
    network: u8,
    account: u32,
    wallet_id: String,
    encryption: ProvisionedEncryption,
    identity_public_key: String,
    spark_address: String,
}

impl ProvisionBlob {
    /// Rejects a blob whose layout version or wallet binding does not match this
    /// config, so a stale or mispaired blob triggers a re-provision instead of
    /// building a signer against the wrong keys.
    fn ensure_usable(&self, network: u8, account: u32, wallet_id: &str) -> Result<(), SignerError> {
        if self.version != PROVISION_VERSION {
            return Err(SignerError::ProvisioningOutdated(format!(
                "provisioned state version {} is not {PROVISION_VERSION}; re-provision",
                self.version
            )));
        }
        if self.network != network || self.account != account || self.wallet_id != wallet_id {
            return Err(SignerError::ProvisioningOutdated(
                "provisioned state does not match this config; re-provision".to_string(),
            ));
        }
        Ok(())
    }
}

/// Persistable result of provisioning a Turnkey wallet for SDK use.
///
/// Opaque bytes holding either a scoped secret (a non-Spark key used only for
/// local ECIES/HMAC, never funds or the Spark identity) or a record that the
/// wallet denies export, plus the wallet binding. Store it encrypted, once, at
/// user creation, then pass it to [`create_turnkey_signer`] on every later init
/// to build the signer with no network calls. Re-provision when a later
/// `create_turnkey_signer` rejects it as outdated (an SDK upgrade), or after
/// changing the wallet's export policy.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TurnkeyProvisionedSigner {
    pub bytes: Vec<u8>,
}

/// One-time setup for a Turnkey-backed wallet, to run once at user creation.
///
/// Materializes the enclave identity account and exports the SDK-layer
/// encryption key, returning a [`TurnkeyProvisionedSigner`] to persist. Later
/// inits pass it to [`create_turnkey_signer`] to build the signer with no
/// network. Idempotent, so it is safe to re-run if the persisted result is lost
/// or after a policy or SDK-version change.
///
/// A wallet whose policy denies key export still provisions: the export's 403 is
/// recorded as unavailable, and the built signer reports encryption unavailable
/// without ever attempting the export.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn provision_turnkey_signer(
    config: TurnkeyConfig,
) -> Result<TurnkeyProvisionedSigner, SignerError> {
    let account = account_number(&config);
    let http = create_http_client(Some("breez-sdk-spark-turnkey"));
    let client = TurnkeyClient::new(&config, http).map_err(to_signer_err)?;

    // Materialize the compressed identity account (idempotent). Its address is
    // the compressed identity pubkey hex, which `create` seeds into the Spark
    // signer so a network-free build can sign ECDSA identity messages.
    let identity_public_key = client
        .create_account(identity_path(account), ADDRESS_FORMAT_COMPRESSED)
        .await
        .map_err(to_signer_err)?;
    let identity_pubkey = PublicKey::from_str(&identity_public_key).map_err(to_signer_err)?;

    // Materialize the Spark-format account at the same path so enclave Schnorr
    // and Spark-protocol signing work; the address itself is derived locally (the
    // canonical Spark address for the identity key), matching the Spark signer
    // and avoiding the ambiguous get-by-path once both formats exist.
    client
        .create_account(identity_path(account), spark_address_format(config.network))
        .await
        .map_err(to_signer_err)?;
    let spark_address = SparkAddress::new(identity_pubkey, config.network.into(), None)
        .to_address_string()
        .map_err(to_signer_err)?;

    // Export the dedicated ECIES/HMAC key. A deny-export policy (403) is a
    // definitive verdict recorded in the blob, so `create` never re-probes.
    let encryption = match client
        .export_secret_key(encryption_key_path(account), ADDRESS_FORMAT_COMPRESSED)
        .await
    {
        Ok(secret) => ProvisionedEncryption::Key(secret.secret_bytes()),
        Err(TurnkeyError::Http { status: 403, .. }) => ProvisionedEncryption::Unavailable,
        Err(e) => return Err(to_signer_err(e)),
    };

    let blob = ProvisionBlob {
        version: PROVISION_VERSION,
        network: config.network as u8,
        account,
        wallet_id: config.wallet_id.clone(),
        encryption,
        identity_public_key,
        spark_address,
    };
    Ok(TurnkeyProvisionedSigner {
        bytes: serde_json::to_vec(&blob).map_err(to_signer_err)?,
    })
}

/// Builds the Turnkey-backed Breez and Spark signers from `config`, sharing one
/// Turnkey client.
///
/// With `provisioned` from a prior [`provision_turnkey_signer`], this makes no
/// network calls: the blob attests the identity account exists and carries the
/// encryption-key verdict (a seeded key, or that export is denied). A blob that
/// does not match `config` (wallet, network, account, or an older layout) is
/// rejected with [`SignerError::ProvisioningOutdated`] so the caller
/// re-provisions.
///
/// Without `provisioned` (mobile/CLI), the identity account is materialized
/// eagerly and the encryption key is exported lazily on first ECIES/HMAC use.
///
/// The Spark signer keeps every signing operation in the Turnkey enclave; the
/// Breez signer does too, except ECIES and HMAC, which run locally against a
/// dedicated, non-Spark key. Using a non-Spark key keeps every Spark key (the
/// identity key included) in the enclave; ECIES/HMAC only need a stable key.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn create_turnkey_signer(
    config: TurnkeyConfig,
    provisioned: Option<TurnkeyProvisionedSigner>,
) -> Result<ExternalSigners, SignerError> {
    let network = config.network;
    let account = account_number(&config);
    let http = create_http_client(Some("breez-sdk-spark-turnkey"));
    let client = Arc::new(TurnkeyClient::new(&config, http).map_err(to_signer_err)?);

    // `encryption` seeds the Breez signer's ECIES/HMAC backend; `spark_identity`
    // seeds the Spark signer's identity pubkey and Spark address so a provisioned
    // build makes no Turnkey calls for either.
    let (encryption, spark_identity) = match provisioned {
        // Unprovisioned: materialize the identity account now, export lazily, and
        // let the Spark signer fetch its identity/address on first use.
        None => {
            client
                .create_account(identity_path(account), ADDRESS_FORMAT_COMPRESSED)
                .await
                .map_err(to_signer_err)?;
            (EncryptionBackend::Lazy, None)
        }
        // Provisioned once: no network. The blob attests the accounts exist and
        // carries the encryption verdict plus the stable Spark identity values.
        Some(provisioned) => {
            let blob: ProvisionBlob = serde_json::from_slice(&provisioned.bytes).map_err(|e| {
                SignerError::ProvisioningOutdated(format!("unreadable provisioned state: {e}"))
            })?;
            blob.ensure_usable(network as u8, account, &config.wallet_id)?;
            let encryption = match blob.encryption {
                ProvisionedEncryption::Key(bytes) => {
                    let secret = SecretKey::from_slice(&bytes).map_err(to_signer_err)?;
                    EncryptionBackend::Seeded(BreezSignerImpl::new(xpriv_from_secret(
                        secret, network,
                    )))
                }
                ProvisionedEncryption::Unavailable => {
                    EncryptionBackend::Denied("Turnkey wallet policy denies key export".to_string())
                }
            };
            let identity_pubkey =
                PublicKey::from_str(&blob.identity_public_key).map_err(to_signer_err)?;
            (encryption, Some((identity_pubkey, blob.spark_address)))
        }
    };

    let breez_signer: Arc<dyn ExternalBreezSigner> = Arc::new(TurnkeyBreezSigner::new_seeded(
        client.clone(),
        network,
        account,
        encryption,
    ));
    let spark_signer: Arc<dyn ExternalSparkSigner> = Arc::new(match spark_identity {
        Some((identity_pubkey, spark_address)) => TurnkeySparkSigner::new_seeded(
            client,
            network,
            account,
            Some(identity_pubkey),
            Some(spark_address),
        ),
        None => TurnkeySparkSigner::new(client, network, account),
    });
    Ok(ExternalSigners {
        breez_signer,
        spark_signer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;
    use bitcoin::secp256k1::Secp256k1;

    // Unroutable base URL: every test here exercises the provisioned path, which
    // must not touch the network, so a real request would be a bug (and hang).
    fn test_config() -> TurnkeyConfig {
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
            retry: None,
        }
    }

    // A fixed, valid identity pubkey to seed provisioned blobs with.
    fn test_identity_pubkey() -> PublicKey {
        let secp = Secp256k1::new();
        SecretKey::from_slice(&[0x22; 32])
            .unwrap()
            .public_key(&secp)
    }

    fn provisioned(
        config: &TurnkeyConfig,
        encryption: ProvisionedEncryption,
    ) -> TurnkeyProvisionedSigner {
        let blob = ProvisionBlob {
            version: PROVISION_VERSION,
            network: config.network as u8,
            account: account_number(config),
            wallet_id: config.wallet_id.clone(),
            encryption,
            identity_public_key: hex::encode(test_identity_pubkey().serialize()),
            spark_address: "sprt1qqtestsparkaddress".to_string(),
        };
        TurnkeyProvisionedSigner {
            bytes: serde_json::to_vec(&blob).unwrap(),
        }
    }

    // A seeded blob builds a signer whose ECIES runs locally: encrypt/decrypt
    // round-trips with no network (the base URL is unroutable).
    #[tokio::test]
    async fn seeded_blob_builds_offline_encryptor() {
        let config = test_config();
        let state = provisioned(&config, ProvisionedEncryption::Key([7u8; 32]));
        let signers = create_turnkey_signer(config, Some(state)).await.unwrap();

        let message = vec![1, 2, 3, 4];
        let ciphertext = signers
            .breez_signer
            .encrypt_ecies(message.clone(), "m/0'".to_string())
            .await
            .unwrap();
        let plaintext = signers
            .breez_signer
            .decrypt_ecies(ciphertext, "m/0'".to_string())
            .await
            .unwrap();
        assert_eq!(plaintext, message);
    }

    // A seeded blob serves the Spark identity pubkey from memory: it returns the
    // provisioned key with no network (the base URL is unroutable, so a fetch
    // would fail), matching what the whole-signer in-memory cache did.
    #[tokio::test]
    async fn seeded_blob_serves_spark_identity_offline() {
        let config = test_config();
        let state = provisioned(&config, ProvisionedEncryption::Key([7u8; 32]));
        let signers = create_turnkey_signer(config, Some(state)).await.unwrap();

        let identity = signers
            .spark_signer
            .get_identity_public_key()
            .await
            .unwrap();
        assert_eq!(identity.to_public_key().unwrap(), test_identity_pubkey());
    }

    // A blob recording denied export builds a signer that reports encryption
    // unavailable without attempting the export (no network).
    #[tokio::test]
    async fn unavailable_blob_reports_encryption_unavailable() {
        let config = test_config();
        let state = provisioned(&config, ProvisionedEncryption::Unavailable);
        let signers = create_turnkey_signer(config, Some(state)).await.unwrap();

        match signers
            .breez_signer
            .encrypt_ecies(vec![1], "m/0'".to_string())
            .await
        {
            Err(SignerError::EncryptionUnavailable(_)) => {}
            other => panic!("expected EncryptionUnavailable, got {other:?}"),
        }
    }

    // A blob for a different wallet is rejected rather than used against the
    // wrong keys.
    #[tokio::test]
    async fn mismatched_blob_is_rejected() {
        let config = test_config();
        let mut other = test_config();
        other.wallet_id = "other-wallet".to_string();
        let state = provisioned(&other, ProvisionedEncryption::Key([7u8; 32]));

        match create_turnkey_signer(config, Some(state)).await {
            Err(SignerError::ProvisioningOutdated(_)) => {}
            result => panic!(
                "expected ProvisioningOutdated, got {:?}",
                result.map(|_| ())
            ),
        }
    }

    // An older layout version is rejected as outdated so the caller re-provisions.
    #[tokio::test]
    async fn outdated_version_is_rejected() {
        let config = test_config();
        let old_blob = ProvisionBlob {
            version: PROVISION_VERSION - 1,
            network: config.network as u8,
            account: account_number(&config),
            wallet_id: config.wallet_id.clone(),
            encryption: ProvisionedEncryption::Unavailable,
            identity_public_key: hex::encode(test_identity_pubkey().serialize()),
            spark_address: "sprt1qqtestsparkaddress".to_string(),
        };
        let state = TurnkeyProvisionedSigner {
            bytes: serde_json::to_vec(&old_blob).unwrap(),
        };

        match create_turnkey_signer(config, Some(state)).await {
            Err(SignerError::ProvisioningOutdated(_)) => {}
            result => panic!(
                "expected ProvisioningOutdated, got {:?}",
                result.map(|_| ())
            ),
        }
    }
}
