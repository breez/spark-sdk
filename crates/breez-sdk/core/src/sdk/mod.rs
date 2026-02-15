mod api;
mod deposits;
mod helpers;
mod init;
mod lightning_address;
mod lnurl;
mod payments;
mod sync;
mod unified_payment;

use bitflags::bitflags;
use breez_sdk_common::{buy::BuyBitcoinProviderApi, fiat::FiatService, rest::RestClient};
use spark_wallet::SparkWallet;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell, oneshot, watch};
use tokio_with_wasm::alias as tokio;

use crate::{
    BitcoinChainService, ExternalInputParser, InputType, Logger, Network, OptimizationConfig,
    error::SdkError, events::EventEmitter, lnurl::LnurlServerClient, logger, models::Config,
    nostr::NostrClient, persist::Storage, token_conversion::TokenConverter,
};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub(crate) const BREEZ_SYNC_SERVICE_URL: &str = "https://datasync.breez.technology";

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub(crate) const BREEZ_SYNC_SERVICE_URL: &str = "https://datasync.breez.technology:442";

pub(crate) const CLAIM_TX_SIZE_VBYTES: u64 = 99;
pub(crate) const SYNC_PAGING_LIMIT: u32 = 100;

bitflags! {
    #[derive(Clone, Debug)]
    pub(crate) struct SyncType: u32 {
        const Wallet = 1 << 0;
        const WalletState = 1 << 1;
        const Deposits = 1 << 2;
        const LnurlMetadata = 1 << 3;
        const Full = Self::Wallet.0.0
            | Self::WalletState.0.0
            | Self::Deposits.0.0
            | Self::LnurlMetadata.0.0;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SyncRequest {
    pub(crate) sync_type: SyncType,
    #[allow(clippy::type_complexity)]
    pub(crate) reply: Arc<Mutex<Option<oneshot::Sender<Result<(), SdkError>>>>>,
    /// If true, bypass the "recently synced" check and sync immediately.
    /// Use for event-driven syncs (after payments, transfers, etc.) that should happen immediately.
    pub(crate) force: bool,
}

impl SyncRequest {
    pub(crate) fn full(reply: Option<oneshot::Sender<Result<(), SdkError>>>) -> Self {
        Self {
            sync_type: SyncType::Full,
            reply: Arc::new(Mutex::new(reply)),
            force: true,
        }
    }

    pub(crate) fn no_reply(sync_type: SyncType) -> Self {
        Self {
            sync_type,
            reply: Arc::new(Mutex::new(None)),
            force: true,
        }
    }

    /// For timer-based periodic syncs that respect the debounce interval.
    pub(crate) fn periodic() -> Self {
        Self {
            sync_type: SyncType::Full,
            reply: Arc::new(Mutex::new(None)),
            force: false,
        }
    }

    pub(crate) async fn reply(&self, error: Option<SdkError>) {
        if let Some(reply) = self.reply.lock().await.take() {
            let _ = match error {
                Some(e) => reply.send(Err(e)),
                None => reply.send(Ok(())),
            };
        }
    }
}

/// A per-seed wallet instance.
///
/// Created via [`App::connect_wallet`](crate::App::connect_wallet) (new API)
/// or [`connect`] (legacy API).
///
/// Holds live wallet state and all payment/query operations.
///
/// This is a type alias for backward compatibility — [`BreezSdk`] and `Wallet`
/// are the same type.
pub type Wallet = BreezSdk;

/// `BreezSDK` is a wrapper around `SparkSDK` that provides a more structured API
/// with request/response objects and comprehensive error handling.
///
/// **Note:** For new code, prefer using [`Wallet`] (which is just an alias for this type)
/// obtained via [`App::connect_wallet`](crate::App::connect_wallet).
#[derive(Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BreezSdk {
    pub(crate) config: Config,
    pub(crate) spark_wallet: Arc<SparkWallet>,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) chain_service: Arc<dyn BitcoinChainService>,
    pub(crate) fiat_service: Arc<dyn FiatService>,
    pub(crate) lnurl_client: Arc<dyn RestClient>,
    pub(crate) lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    pub(crate) lnurl_auth_signer: Arc<crate::signer::lnurl_auth::LnurlAuthSignerAdapter>,
    pub(crate) event_emitter: Arc<EventEmitter>,
    pub(crate) shutdown_sender: watch::Sender<()>,
    pub(crate) sync_trigger: tokio::sync::broadcast::Sender<SyncRequest>,
    pub(crate) zap_receipt_trigger: tokio::sync::broadcast::Sender<()>,
    pub(crate) initial_synced_watcher: watch::Receiver<bool>,
    pub(crate) external_input_parsers: Vec<ExternalInputParser>,
    pub(crate) spark_private_mode_initialized: Arc<OnceCell<()>>,
    pub(crate) nostr_client: Arc<NostrClient>,
    pub(crate) token_converter: Arc<dyn TokenConverter>,
    pub(crate) buy_bitcoin_provider: Arc<dyn BuyBitcoinProviderApi>,
}

pub(crate) struct BreezSdkParams {
    pub config: Config,
    pub storage: Arc<dyn Storage>,
    pub chain_service: Arc<dyn BitcoinChainService>,
    pub fiat_service: Arc<dyn FiatService>,
    pub lnurl_client: Arc<dyn RestClient>,
    pub lnurl_server_client: Option<Arc<dyn LnurlServerClient>>,
    pub lnurl_auth_signer: Arc<crate::signer::lnurl_auth::LnurlAuthSignerAdapter>,
    pub shutdown_sender: watch::Sender<()>,
    pub spark_wallet: Arc<SparkWallet>,
    pub event_emitter: Arc<EventEmitter>,
    pub nostr_client: Arc<NostrClient>,
    pub buy_bitcoin_provider: Arc<dyn BuyBitcoinProviderApi>,
}

pub async fn parse_input(
    input: &str,
    external_input_parsers: Option<Vec<ExternalInputParser>>,
) -> Result<InputType, SdkError> {
    Ok(breez_sdk_common::input::parse(
        input,
        external_input_parsers.map(|parsers| parsers.into_iter().map(From::from).collect()),
    )
    .await?
    .into())
}

/// Verify a message signature without requiring a wallet connection.
///
/// This is a pure secp256k1 ECDSA verification — no wallet state needed.
pub fn verify_message(
    request: crate::CheckMessageRequest,
) -> Result<crate::CheckMessageResponse, SdkError> {
    use bitcoin::hashes::Hash as _;
    use bitcoin::secp256k1::{Secp256k1, ecdsa::Signature};
    use std::str::FromStr;

    let pubkey = bitcoin::secp256k1::PublicKey::from_str(&request.pubkey)
        .map_err(|_| SdkError::InvalidInput("Invalid public key".to_string()))?;
    let signature_bytes = hex::decode(&request.signature)
        .map_err(|_| SdkError::InvalidInput("Not a valid hex encoded signature".to_string()))?;
    let signature = Signature::from_der(&signature_bytes)
        .or_else(|_| Signature::from_compact(&signature_bytes))
        .map_err(|_| {
            SdkError::InvalidInput("Not a valid DER or compact encoded signature".to_string())
        })?;

    let digest = bitcoin::hashes::sha256::Hash::hash(request.message.as_bytes());
    let msg = bitcoin::secp256k1::Message::from_digest(digest.to_byte_array());
    let is_valid = Secp256k1::new()
        .verify_ecdsa(&msg, &signature, &pubkey)
        .is_ok();

    Ok(crate::CheckMessageResponse { is_valid })
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn init_logging(
    log_dir: Option<String>,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<String>,
) -> Result<(), SdkError> {
    logger::init_logging(log_dir, app_logger, log_filter)
}

/// Connects to the Spark network using the provided configuration and mnemonic.
///
/// # Deprecated
///
/// Use [`App::new`](crate::App::new) + [`App::connect_wallet`](crate::App::connect_wallet) instead:
///
/// ```ignore
/// let app = App::new(AppConfig { api_key: "..".into(), network: Network::Mainnet, ..Default::default() })?;
/// let wallet = app.connect_wallet(WalletConfig { seed, ..Default::default() }).await?;
/// ```
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[deprecated(note = "Use App::new() + app.connect_wallet() instead")]
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn connect(request: crate::ConnectRequest) -> Result<BreezSdk, SdkError> {
    let builder = super::sdk_builder::SdkBuilder::new(request.config, request.seed)
        .with_default_storage(request.storage_dir);
    let sdk = builder.build().await?;
    Ok(sdk)
}

/// Connects to the Spark network using an external signer.
///
/// # Deprecated
///
/// Use [`App::new`](crate::App::new) + [`App::connect_wallet`](crate::App::connect_wallet) instead.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[deprecated(note = "Use App::new() + app.connect_wallet() instead")]
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn connect_with_signer(
    request: crate::ConnectWithSignerRequest,
) -> Result<BreezSdk, SdkError> {
    let builder = super::sdk_builder::SdkBuilder::new_with_signer(request.config, request.signer)
        .with_default_storage(request.storage_dir);
    let sdk = builder.build().await?;
    Ok(sdk)
}

/// # Deprecated
///
/// Use [`AppConfig`](crate::AppConfig) with `..Default::default()` instead.
#[deprecated(note = "Use AppConfig with defaults instead")]
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_config(network: Network) -> Config {
    let lnurl_domain = match network {
        Network::Mainnet => Some("breez.tips".to_string()),
        Network::Regtest => None,
    };
    Config {
        api_key: None,
        network,
        sync_interval_secs: 60, // every 1 minute
        max_deposit_claim_fee: Some(crate::MaxFee::Rate { sat_per_vbyte: 1 }),
        lnurl_domain,
        prefer_spark_over_lightning: false,
        external_input_parsers: None,
        use_default_external_input_parsers: true,
        real_time_sync_server_url: Some(BREEZ_SYNC_SERVICE_URL.to_string()),
        private_enabled_default: true,
        optimization_config: OptimizationConfig {
            auto_enabled: true,
            multiplicity: 1,
        },
    }
}

/// Creates a default external signer from a mnemonic.
///
/// This is a convenience factory method for creating a signer that can be used
/// with `connect_with_signer` or `SdkBuilder::new_with_signer`.
///
/// # Arguments
///
/// * `mnemonic` - BIP39 mnemonic phrase (12 or 24 words)
/// * `passphrase` - Optional passphrase for the mnemonic
/// * `network` - Network to use (Mainnet or Regtest)
/// * `key_set_config` - Optional key set configuration. If None, uses default configuration.
///
/// # Returns
///
/// Result containing the signer as `Arc<dyn ExternalSigner>`
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_external_signer(
    mnemonic: String,
    passphrase: Option<String>,
    network: Network,
    key_set_config: Option<crate::models::KeySetConfig>,
) -> Result<Arc<dyn crate::signer::ExternalSigner>, SdkError> {
    use crate::signer::DefaultExternalSigner;

    let config = key_set_config.unwrap_or_default();
    let signer = DefaultExternalSigner::new(
        mnemonic,
        passphrase,
        network,
        config.key_set_type,
        config.use_address_index,
        config.account_number,
    )?;

    Ok(Arc::new(signer))
}

/// Fetches the current status of Spark network services relevant to the SDK.
///
/// This function queries the Spark status API and returns the worst status
/// across the Spark Operators and SSP services.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn get_spark_status() -> Result<crate::SparkStatus, SdkError> {
    use breez_sdk_common::rest::ReqwestRestClient;
    use chrono::DateTime;

    #[derive(serde::Deserialize)]
    struct StatusApiResponse {
        services: Vec<StatusApiService>,
        #[serde(rename = "lastUpdated")]
        last_updated: String,
    }

    #[derive(serde::Deserialize)]
    struct StatusApiService {
        name: String,
        status: String,
    }

    fn parse_service_status(s: &str) -> crate::ServiceStatus {
        match s {
            "operational" => crate::ServiceStatus::Operational,
            "degraded" => crate::ServiceStatus::Degraded,
            "partial" => crate::ServiceStatus::Partial,
            "major" => crate::ServiceStatus::Major,
            _ => {
                tracing::warn!("Unknown service status: {s}");
                crate::ServiceStatus::Unknown
            }
        }
    }

    let rest_client =
        ReqwestRestClient::new().map_err(|e| SdkError::NetworkError(e.to_string()))?;

    let response = rest_client
        .get_request("https://spark.money/api/v1/status".to_string(), None)
        .await
        .map_err(|e| SdkError::NetworkError(e.to_string()))?;

    let api_response: StatusApiResponse = serde_json::from_str(&response.body)
        .map_err(|e| SdkError::Generic(format!("Failed to parse status response: {e}")))?;

    let status = api_response
        .services
        .iter()
        .filter(|s| s.name == "Spark Operators" || s.name == "SSP")
        .map(|s| parse_service_status(&s.status))
        .max()
        .unwrap_or(crate::ServiceStatus::Unknown);

    let last_updated = DateTime::parse_from_rfc3339(&api_response.last_updated)
        .map(|dt| dt.timestamp().cast_unsigned())
        .map_err(|e| SdkError::Generic(format!("Failed to parse lastUpdated timestamp: {e}")))?;

    Ok(crate::SparkStatus {
        status,
        last_updated,
    })
}
