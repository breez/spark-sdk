//! Factory for the Turnkey-backed signers, exposed over uniffi.

use std::sync::Arc;

use platform_utils::create_http_client;

use crate::error::SignerError;
use crate::signer::breez::BreezSignerImpl;
use crate::signer::{ExternalBreezSigner, ExternalSparkSigner};

use super::accounts::{spark_address_format, xpriv_from_secret};
use super::breez_signer::TurnkeyBreezSigner;
use super::config::TurnkeyConfig;
use super::spark_signer::TurnkeySparkSigner;
use super::transport::TurnkeyClient;

fn to_signer_err<E: std::fmt::Display>(e: E) -> SignerError {
    SignerError::Generic(e.to_string())
}

/// The two Turnkey-backed signers, ready to pass to the SDK's signer-based
/// connect: `breez` for non-Spark SDK signing, `spark` for the Spark wallet.
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TurnkeySigners {
    pub breez: Arc<dyn ExternalBreezSigner>,
    pub spark: Arc<dyn ExternalSparkSigner>,
}

/// Builds the Turnkey-backed Breez and Spark signers from `config`, sharing one
/// Turnkey client.
///
/// The Spark signer keeps every signing operation in the Turnkey enclave; the
/// Breez signer does too, except ECIES and HMAC, which run locally against the
/// wallet's identity key (exported once here). Exporting the identity key
/// matches what a seed-based signer derives, so multi-device sync and
/// LNURL-auth stay consistent.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub async fn create_turnkey_signer(config: TurnkeyConfig) -> Result<TurnkeySigners, SignerError> {
    let network = config.network;
    let http = create_http_client(Some("breez-sdk-spark-turnkey"));
    let client = Arc::new(TurnkeyClient::new(&config, http).map_err(to_signer_err)?);
    // Account-0 identity path in Spark format, matching the Spark signer's
    // identity account: a path can hold only one account, so both use Spark and
    // read the compressed pubkey from its publicKey field.
    let encryption_key = client
        .export_secret_key(
            "m/8797555'/0'/0'".to_string(),
            spark_address_format(network),
        )
        .await
        .map_err(to_signer_err)?;
    let encryption = BreezSignerImpl::new(xpriv_from_secret(encryption_key, network));
    let breez: Arc<dyn ExternalBreezSigner> =
        Arc::new(TurnkeyBreezSigner::new(client.clone(), network, encryption));
    let spark: Arc<dyn ExternalSparkSigner> = Arc::new(TurnkeySparkSigner::new(client, network));
    Ok(TurnkeySigners { breez, spark })
}
