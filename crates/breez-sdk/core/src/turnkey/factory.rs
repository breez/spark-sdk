//! Factory for the Turnkey-backed signers, exposed over uniffi.

use std::sync::Arc;

use platform_utils::create_http_client;

use crate::ExternalSigners;
use crate::error::SignerError;
use crate::signer::breez::BreezSignerImpl;
use crate::signer::{ExternalBreezSigner, ExternalSparkSigner};

use super::accounts::xpriv_from_secret;
use super::breez_signer::TurnkeyBreezSigner;
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

/// Dedicated SDK-layer encryption key path. The Spark signer only derives the
/// account's low-index children (identity/signing/deposit/static/preimage), so
/// this reserved max-index child is never a Spark key and can be exported to
/// seed local ECIES/HMAC without exposing any Spark key.
fn encryption_key_path(account: u32) -> String {
    format!("m/8797555'/{account}'/2147483647'")
}

/// Builds the Turnkey-backed Breez and Spark signers from `config`, sharing one
/// Turnkey client.
///
/// The Spark signer keeps every signing operation in the Turnkey enclave; the
/// Breez signer does too, except ECIES and HMAC, which run locally against a
/// dedicated, non-Spark key exported once here. Exporting a non-Spark key keeps
/// every Spark key (the identity key included) in the enclave; ECIES/HMAC only
/// need a stable key, not a Spark one.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn create_turnkey_signer(config: TurnkeyConfig) -> Result<ExternalSigners, SignerError> {
    let network = config.network;
    let account = account_number(&config);
    let http = create_http_client(Some("breez-sdk-spark-turnkey"));
    let client = Arc::new(TurnkeyClient::new(&config, http).map_err(to_signer_err)?);
    // Materialize the compressed identity account so ECDSA identity signing
    // (operator auth, messages) can use it as signWith; the key stays in the
    // enclave (the Spark signer adds the Spark-format account at the same path).
    client
        .create_account(identity_path(account), ADDRESS_FORMAT_COMPRESSED)
        .await
        .map_err(to_signer_err)?;
    // Export a dedicated, non-Spark key to seed the local ECIES/HMAC signer, so
    // no Spark key ever leaves the enclave.
    let encryption_key = client
        .export_secret_key(encryption_key_path(account), ADDRESS_FORMAT_COMPRESSED)
        .await
        .map_err(to_signer_err)?;
    let encryption = BreezSignerImpl::new(xpriv_from_secret(encryption_key, network));
    let breez_signer: Arc<dyn ExternalBreezSigner> = Arc::new(TurnkeyBreezSigner::new(
        client.clone(),
        network,
        account,
        encryption,
    ));
    let spark_signer: Arc<dyn ExternalSparkSigner> =
        Arc::new(TurnkeySparkSigner::new(client, network, account));
    Ok(ExternalSigners {
        breez_signer,
        spark_signer,
    })
}
