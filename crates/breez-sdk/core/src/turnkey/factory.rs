//! Factory for the Turnkey-backed signers, exposed over uniffi.

use std::sync::Arc;

use platform_utils::create_http_client;

use crate::error::SignerError;
use crate::signer::{ExternalBreezSigner, ExternalSigningSigner, ExternalSparkSigner};
use crate::{ExternalSigners, Network, SigningOnlyExternalSigners};

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

/// Sets up the shared Turnkey client and materializes the compressed identity
/// account so ECDSA identity signing (operator auth, messages) can use it as
/// signWith; the key stays in the enclave (the Spark signer adds the
/// Spark-format account at the same path).
async fn build_client(
    config: &TurnkeyConfig,
) -> Result<(Arc<TurnkeyClient>, Network, u32), SignerError> {
    let network = config.network;
    let account = account_number(config);
    let http = create_http_client(Some("breez-sdk-spark-turnkey"));
    let client = Arc::new(TurnkeyClient::new(config, http).map_err(to_signer_err)?);
    client
        .create_account(identity_path(account), ADDRESS_FORMAT_COMPRESSED)
        .await
        .map_err(to_signer_err)?;
    Ok((client, network, account))
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
        Arc::new(TurnkeySparkSigner::new(client, network, account));
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
        Arc::new(TurnkeySparkSigner::new(client, network, account));
    Ok(SigningOnlyExternalSigners {
        breez_signer,
        spark_signer,
    })
}
