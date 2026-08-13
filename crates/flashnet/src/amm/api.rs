use std::{collections::HashMap, sync::Arc};

use bitcoin::hashes::{Hash, sha256};
use spark::bech32m_encode_token_id;
use spark_wallet::{PublicKey, SparkAddress, SparkWallet, TokenRecipient, TransferId};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::models::{
    ClawbackIntent, ClawbackRequest, ClawbackResponse, ExecuteSwapIntent, ExecuteSwapRequest,
    ExecuteSwapResponse, FeatureName, FeatureStatus, FlashnetExecuteSwapResponse,
    GetMinAmountsRequest, GetMinAmountsResponse, ListClawbackTransfersRequest,
    ListClawbackTransfersResponse, ListPoolsRequest, ListPoolsResponse, ListUserSwapsRequest,
    ListUserSwapsResponse, MinAmount, PingResponse, Pool, PreparedSwap, SignedClawbackRequest,
    SignedExecuteSwapRequest, SignedExecuteSwapResponse, SimulateSwapRequest, SimulateSwapResponse,
    SwapDegradation, SwapOutcome,
};
use super::utils::generate_nonce;
use crate::cache::CacheStore;
use crate::config::{FlashnetConfig, IntegratorConfig};
use crate::error::FlashnetError;
use crate::models::AssetTransfer;

pub const BTC_ASSET_ADDRESS: &str =
    "020202020202020202020202020202020202020202020202020202020202020202";

const FEATURE_STATUSES_CACHE_KEY: &str = "feature_statuses";
const LIST_POOLS_CACHE_KEY: &str = "list_pools";
const MIN_AMOUNTS_CACHE_KEY: &str = "min_amounts";
const PING_CACHE_KEY: &str = "ping";
const FEATURE_STATUSES_TTL_MS: u32 = 5_000;
const LIST_POOLS_TTL_MS: u32 = 60_000;
const MIN_AMOUNTS_TTL_MS: u32 = 5_000;
const PING_TTL_MS: u32 = 2_000;
/// An integrator fee above this would exceed the principal being swapped.
const MAX_INTEGRATOR_FEE_BPS: u32 = 10_000;

/// The integrator to credit and the rate to charge them. An override replaces
/// the configured integrator outright rather than merging with it, so a rate
/// never arrives without a recipient.
fn resolve_integrator(
    override_config: Option<&IntegratorConfig>,
    configured: Option<&IntegratorConfig>,
) -> Result<(u32, Option<PublicKey>), FlashnetError> {
    let Some(integrator) = override_config.or(configured) else {
        return Ok((0, None));
    };
    if integrator.fee_bps > MAX_INTEGRATOR_FEE_BPS {
        return Err(FlashnetError::Generic(format!(
            "Integrator fee {} bps exceeds {MAX_INTEGRATOR_FEE_BPS}",
            integrator.fee_bps
        )));
    }
    Ok((integrator.fee_bps, Some(integrator.pubkey)))
}

pub struct FlashnetClient {
    pub(crate) config: FlashnetConfig,
    pub(crate) cache_store: Arc<CacheStore>,
    pub(crate) spark_wallet: Arc<SparkWallet>,
    pub(crate) http_client: Arc<dyn platform_utils::HttpClient>,
    /// Mutex to serialize authentication attempts and prevent race conditions
    pub(crate) auth_mutex: Mutex<()>,
}

impl FlashnetClient {
    pub fn new(
        config: FlashnetConfig,
        spark_wallet: Arc<SparkWallet>,
        cache_store: Arc<CacheStore>,
        http_client: Arc<dyn platform_utils::HttpClient>,
    ) -> Self {
        Self {
            config,
            cache_store,
            spark_wallet,
            http_client,
            auth_mutex: Mutex::new(()),
        }
    }

    pub async fn clawback(
        &self,
        request: ClawbackRequest,
    ) -> Result<ClawbackResponse, FlashnetError> {
        debug!("Clawback request: {:?}", request);

        // Pre-checks
        self.ensure_ping_ok().await?;

        self.sign_clawback(request).await
    }

    pub async fn list_clawback_transfers(
        &self,
        request: ListClawbackTransfersRequest,
    ) -> Result<ListClawbackTransfersResponse, FlashnetError> {
        debug!("List clawback transfers request: {request:?}");
        self.get_request("v1/clawback-transfers/list", Some(request))
            .await
    }

    pub async fn get_min_amounts(
        &self,
        request: GetMinAmountsRequest,
    ) -> Result<GetMinAmountsResponse, FlashnetError> {
        let request = request.decode_token_identifiers(self.config.network)?;
        debug!("Get limits request: {request:?}");
        let min_amounts = self.config_min_amounts().await?;
        let min_amounts_map = min_amounts
            .into_iter()
            .filter(|ma| ma.enabled)
            .map(|ma| (ma.asset_identifier, ma.min_amount))
            .collect::<HashMap<_, _>>();
        if let Some(min_in) = min_amounts_map.get(&request.asset_in_address) {
            return Ok(GetMinAmountsResponse {
                asset_in_min: Some(*min_in),
                asset_out_min: None,
            });
        } else if let Some(min_out) = min_amounts_map.get(&request.asset_out_address) {
            let relaxed_min_out = min_out.saturating_div(2); // 50% relaxation for slippage
            return Ok(GetMinAmountsResponse {
                asset_in_min: None,
                asset_out_min: Some(relaxed_min_out),
            });
        }

        Ok(GetMinAmountsResponse::default())
    }

    pub async fn get_pool(&self, pool_id: &str) -> Result<ListPoolsResponse, FlashnetError> {
        self.get_request(&format!("v1/pools/{pool_id}"), None::<()>)
            .await
    }

    pub async fn list_pools(
        &self,
        request: ListPoolsRequest,
    ) -> Result<ListPoolsResponse, FlashnetError> {
        let request = request.decode_token_identifiers(self.config.network)?;
        debug!("List pools request: {request:?}");
        // Check cache first for a matching request
        let request_json = serde_json::to_string(&request).map_err(|e| {
            FlashnetError::Generic(format!("Failed to serialize list pools request: {e}"))
        })?;
        let cache_key = format!(
            "{}_{}",
            LIST_POOLS_CACHE_KEY,
            sha256::Hash::hash(request_json.as_bytes())
        );
        if let Some(list_pools_cache) = self
            .cache_store
            .get::<ListPoolsResponse>(&cache_key)
            .await?
        {
            return Ok(list_pools_cache);
        }
        // If it's not in the cache, make an API request
        let response = self.get_request("v1/pools", Some(request)).await?;
        self.cache_store
            .set(&cache_key, &response, LIST_POOLS_TTL_MS.into())
            .await?;
        Ok(response)
    }

    pub async fn list_user_swaps(
        &self,
        request: ListUserSwapsRequest,
    ) -> Result<ListUserSwapsResponse, FlashnetError> {
        let request = request.decode_token_identifiers(self.config.network)?;
        let identity_public_key = self.spark_wallet.get_identity_public_key();
        let endpoint = format!("v1/swaps/user/{identity_public_key}");
        self.get_request(&endpoint, Some(request)).await
    }

    pub async fn simulate_swap(
        &self,
        request: SimulateSwapRequest,
    ) -> Result<SimulateSwapResponse, FlashnetError> {
        let request = request.decode_token_identifiers(self.config.network)?;
        debug!("Simulate swap request: {request:?}");

        // Pre-checks
        self.ensure_ping_ok().await?;
        self.ensure_feature_status(FeatureName::AllowSwaps).await?;
        self.ensure_min_amounts(
            &request.asset_in_address,
            &request.asset_out_address,
            request.amount_in,
            None,
        )
        .await?;

        self.post_request("v1/swap/simulate", request).await
    }

    /// Runs every check and fixes every parameter, without moving funds. Takes
    /// the pool itself rather than an id, so the pool that was validated is the
    /// pool that executes.
    pub async fn prepare_swap(
        &self,
        pool: &Pool,
        request: ExecuteSwapRequest,
    ) -> Result<PreparedSwap, FlashnetError> {
        let request = request.decode_token_identifiers(self.config.network)?;
        debug!("Prepare swap request: {request:?}");

        self.ensure_ping_ok().await?;
        self.ensure_feature_status(FeatureName::AllowSwaps).await?;
        self.ensure_min_amounts(
            &request.asset_in_address,
            &request.asset_out_address,
            request.amount_in,
            Some(request.min_amount_out),
        )
        .await?;

        pool.validate_for_swap(
            &request.asset_in_address,
            &request.asset_out_address,
            self.config.network,
        )?;

        let (total_integrator_fee_rate_bps, integrator_public_key) = resolve_integrator(
            request.integrator_override.as_ref(),
            self.config.integrator_config.as_ref(),
        )?;

        Ok(PreparedSwap {
            pool_id: pool.lp_public_key,
            asset_in_address: request.asset_in_address,
            asset_out_address: request.asset_out_address,
            amount_in: request.amount_in,
            max_slippage_bps: request.max_slippage_bps,
            min_amount_out: request.min_amount_out,
            total_integrator_fee_rate_bps,
            integrator_public_key,
            transfer_id: request.transfer_id,
        })
    }

    /// Moves the input asset to the pool. The returned transfer is the rich
    /// wallet-side object, so a caller can record the sent leg without asking
    /// the operator for it again.
    pub async fn send_swap_input(
        &self,
        prepared: &PreparedSwap,
    ) -> Result<AssetTransfer, FlashnetError> {
        self.transfer_asset(
            prepared.amount_in,
            &prepared.asset_in_address,
            &prepared.pool_id,
            prepared.transfer_id.clone(),
        )
        .await
    }

    /// Signs the intent over the input transfer and submits it, checking the
    /// result against the terms just signed.
    pub async fn finalize_swap(
        &self,
        prepared: &PreparedSwap,
        asset_in_spark_transfer_id: &str,
    ) -> Result<(FlashnetExecuteSwapResponse, SwapOutcome), FlashnetError> {
        debug!("Signing swap execution for: {asset_in_spark_transfer_id}");
        let response = self
            .sign_execute_swap(prepared, asset_in_spark_transfer_id)
            .await?;
        let response = FlashnetExecuteSwapResponse::from_signed_execute_swap_response(
            response,
            asset_in_spark_transfer_id.to_string(),
        );

        // An accepted swap has already run, so a response off the signed terms
        // is recorded rather than refunded. Only a rejection is refundable: a
        // clawback against a consumed transfer is refused, then retried.
        let outcome = match response
            .validate_accepted(prepared.min_amount_out, &prepared.asset_out_address)
        {
            Ok(outcome) => outcome,
            Err(e) => {
                warn!("Swap {asset_in_spark_transfer_id} executed off the signed terms: {e}");
                SwapOutcome::AcceptedDegraded {
                    degradation: SwapDegradation::from(&e),
                }
            }
        };
        Ok((response, outcome))
    }

    /// [`Self::prepare_swap`], [`Self::send_swap_input`] and
    /// [`Self::finalize_swap`] in sequence, for callers with nothing to do
    /// between the transfer and the submission.
    pub async fn execute_swap(
        &self,
        pool: &Pool,
        request: ExecuteSwapRequest,
    ) -> Result<ExecuteSwapResponse, FlashnetError> {
        let prepared = self.prepare_swap(pool, request).await?;
        let outbound_asset_transfer = self.send_swap_input(&prepared).await?;
        let transaction_identifier = outbound_asset_transfer.id();

        match self.finalize_swap(&prepared, &transaction_identifier).await {
            Ok((flashnet_response, outcome)) => Ok(ExecuteSwapResponse {
                flashnet_response,
                outcome,
                outbound_asset_transfer,
            }),
            Err(e) => Err(FlashnetError::execution(e, Some(outbound_asset_transfer))),
        }
    }

    pub(crate) async fn config_feature_status(&self) -> Result<Vec<FeatureStatus>, FlashnetError> {
        self.get_request("v1/config/feature-status", None::<()>)
            .await
    }

    pub(crate) async fn config_min_amounts(&self) -> Result<Vec<MinAmount>, FlashnetError> {
        let min_amounts_cache = self
            .cache_store
            .get::<Vec<MinAmount>>(MIN_AMOUNTS_CACHE_KEY)
            .await?;
        let min_amounts = if let Some(min_amounts) = min_amounts_cache {
            min_amounts
        } else {
            let min_amounts = self
                .get_request("v1/config/min-amounts", None::<()>)
                .await?;
            self.cache_store
                .set(
                    MIN_AMOUNTS_CACHE_KEY,
                    &min_amounts,
                    MIN_AMOUNTS_TTL_MS.into(),
                )
                .await?;
            min_amounts
        };
        Ok(min_amounts)
    }

    pub(crate) async fn ping(&self) -> Result<PingResponse, FlashnetError> {
        self.get_request("v1/ping", None::<()>).await
    }

    async fn ensure_feature_status(&self, feature_name: FeatureName) -> Result<(), FlashnetError> {
        let feature_status_cache = self
            .cache_store
            .get::<Vec<FeatureStatus>>(FEATURE_STATUSES_CACHE_KEY)
            .await?;
        let feature_statuses = if let Some(feature_statuses) = feature_status_cache {
            feature_statuses
        } else {
            let feature_statuses = self.config_feature_status().await?;
            self.cache_store
                .set(
                    FEATURE_STATUSES_CACHE_KEY,
                    &feature_statuses,
                    FEATURE_STATUSES_TTL_MS.into(),
                )
                .await?;
            feature_statuses
        };
        let feature_status = feature_statuses
            .into_iter()
            .find(|fs| fs.feature_name == feature_name);
        match feature_status {
            Some(fs) => {
                if fs.enabled {
                    Ok(())
                } else {
                    Err(FlashnetError::Generic(format!(
                        "Feature {:?} is disabled: {}",
                        feature_name,
                        fs.reason
                            .unwrap_or_else(|| "No reason provided".to_string())
                    )))
                }
            }
            None => Err(FlashnetError::Generic(format!(
                "Feature {feature_name:?} not found in status list"
            ))),
        }
    }

    async fn ensure_min_amounts(
        &self,
        asset_in_address: &str,
        asset_out_address: &str,
        amount_in: u128,
        min_amount_out: Option<u128>,
    ) -> Result<(), FlashnetError> {
        let min_amounts = self
            .get_min_amounts(GetMinAmountsRequest {
                asset_in_address: asset_in_address.to_string(),
                asset_out_address: asset_out_address.to_string(),
            })
            .await?;
        if let Some(min_in) = min_amounts.asset_in_min {
            if amount_in < min_in {
                return Err(FlashnetError::Generic(format!(
                    "Amount in {amount_in} is less than minimum required {min_in} for asset {asset_in_address}",
                )));
            }
            return Ok(());
        }
        if let Some(min_amount_out) = min_amount_out
            && let Some(min_out) = min_amounts.asset_out_min
        {
            if min_amount_out < min_out {
                return Err(FlashnetError::Generic(format!(
                    "Minimum amount out {min_amount_out} is less than required {min_out} (50% relaxed) for asset {asset_out_address}",
                )));
            }
            return Ok(());
        }
        Ok(())
    }

    async fn ensure_ping_ok(&self) -> Result<(), FlashnetError> {
        let ping_cache = self.cache_store.get::<PingResponse>(PING_CACHE_KEY).await?;
        let ping = if let Some(ping) = ping_cache {
            ping
        } else {
            let ping = self.ping().await?;
            self.cache_store
                .set(PING_CACHE_KEY, &ping, PING_TTL_MS.into())
                .await?;
            ping
        };
        if ping.status.to_lowercase() != "ok" {
            return Err(FlashnetError::Generic(
                "Flashnet ping response not ok".to_string(),
            ));
        }
        Ok(())
    }

    async fn sign_clawback(
        &self,
        request: ClawbackRequest,
    ) -> Result<ClawbackResponse, FlashnetError> {
        let nonce = hex::encode(generate_nonce());

        // Construct and sign the intent
        let intent = ClawbackIntent {
            sender_public_key: self.spark_wallet.get_identity_public_key(),
            spark_transfer_id: request.transfer_id.clone(),
            lp_identity_public_key: request.pool_id,
            nonce: nonce.clone(),
        };
        let intent_json = serde_json::to_string(&intent).map_err(|e| {
            FlashnetError::Generic(format!("Failed to serialize execute swap intent: {e}"))
        })?;
        let signature = self.spark_wallet.sign_message(&intent_json).await?;

        // Construct the signed request
        let request = SignedClawbackRequest {
            sender_public_key: self.spark_wallet.get_identity_public_key(),
            spark_transfer_id: request.transfer_id,
            lp_identity_public_key: request.pool_id,
            nonce: intent.nonce,
            signature: hex::encode(signature.serialize_compact()),
        };

        self.post_request("v1/clawback", request).await
    }

    async fn sign_execute_swap(
        &self,
        request: &PreparedSwap,
        asset_in_spark_transfer_id: &str,
    ) -> Result<SignedExecuteSwapResponse, FlashnetError> {
        let nonce = hex::encode(generate_nonce());
        let total_integrator_fee_rate_bps = request.total_integrator_fee_rate_bps;
        let integrator_public_key = request.integrator_public_key;

        // Construct and sign the intent
        let intent = ExecuteSwapIntent {
            user_public_key: self.spark_wallet.get_identity_public_key(),
            lp_identity_public_key: request.pool_id,
            asset_in_spark_transfer_id: asset_in_spark_transfer_id.to_string(),
            asset_in_address: request.asset_in_address.clone(),
            asset_out_address: request.asset_out_address.clone(),
            amount_in: request.amount_in,
            max_slippage_bps: request.max_slippage_bps,
            min_amount_out: request.min_amount_out,
            nonce: nonce.clone(),
            total_integrator_fee_rate_bps,
        };
        let intent_json = serde_json::to_string(&intent).map_err(|e| {
            FlashnetError::Generic(format!("Failed to serialize execute swap intent: {e}"))
        })?;
        let signature = self.spark_wallet.sign_message(&intent_json).await?;

        // Construct the signed request
        let signed_request = SignedExecuteSwapRequest {
            user_public_key: self.spark_wallet.get_identity_public_key(),
            pool_id: request.pool_id,
            asset_in_address: request.asset_in_address.clone(),
            asset_out_address: request.asset_out_address.clone(),
            amount_in: request.amount_in,
            max_slippage_bps: request.max_slippage_bps,
            min_amount_out: request.min_amount_out,
            asset_in_spark_transfer_id: asset_in_spark_transfer_id.to_string(),
            nonce: nonce.clone(),
            total_integrator_fee_rate_bps,
            integrator_public_key: integrator_public_key
                .map(|pk| pk.to_string())
                .unwrap_or_default(),
            signature: hex::encode(signature.serialize_compact()),
        };

        self.post_request("v1/swap", signed_request).await
    }

    async fn transfer_asset(
        &self,
        amount: u128,
        asset_address: &str,
        receiver_public_key: &PublicKey,
        transfer_id: Option<TransferId>,
    ) -> Result<AssetTransfer, FlashnetError> {
        let receiver_address = SparkAddress::new(*receiver_public_key, self.config.network, None);
        if asset_address == BTC_ASSET_ADDRESS {
            // Send a spark transfer
            let transfer = self
                .spark_wallet
                .transfer(
                    u64::try_from(amount).map_err(|e| {
                        FlashnetError::Generic(format!("Failed to convert amount to u64: {e}"))
                    })?,
                    &receiver_address,
                    transfer_id,
                )
                .await?;
            Ok(AssetTransfer::Spark(transfer))
        } else {
            // Send a token transfer
            let asset_address_hex = hex::decode(asset_address).map_err(|e| {
                FlashnetError::Generic(format!("Failed to decode asset address from hex: {e}"))
            })?;
            let token_id = bech32m_encode_token_id(&asset_address_hex, self.config.network)
                .map_err(|e| FlashnetError::Generic(format!("Failed to encode token id: {e}")))?;
            let token_tx = self
                .spark_wallet
                .transfer_tokens(
                    vec![TokenRecipient::Address {
                        token_id,
                        amount,
                        receiver_address,
                    }],
                    None,
                    None,
                )
                .await?;
            Ok(AssetTransfer::Token(token_tx))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_INTEGRATOR_FEE_BPS, resolve_integrator};
    use crate::config::IntegratorConfig;
    use spark_wallet::PublicKey;
    use std::str::FromStr;

    fn key(hex: &str) -> PublicKey {
        PublicKey::from_str(hex).unwrap()
    }

    fn configured() -> IntegratorConfig {
        IntegratorConfig {
            pubkey: key("02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da"),
            fee_bps: 5,
        }
    }

    fn overridden() -> IntegratorConfig {
        IntegratorConfig {
            pubkey: key("0202e9c857e89901e7211897cc0e69be843f865de845f60589614a693f3c966cef"),
            fee_bps: 25,
        }
    }

    #[test]
    fn no_integrator_anywhere_charges_nothing() {
        assert_eq!(resolve_integrator(None, None).unwrap(), (0, None));
    }

    #[test]
    fn the_configured_integrator_applies_when_there_is_no_override() {
        let config = configured();
        assert_eq!(
            resolve_integrator(None, Some(&config)).unwrap(),
            (5, Some(config.pubkey))
        );
    }

    #[test]
    fn an_override_replaces_the_configured_integrator_outright() {
        let (config, over) = (configured(), overridden());
        assert_eq!(
            resolve_integrator(Some(&over), Some(&config)).unwrap(),
            (25, Some(over.pubkey))
        );
    }

    #[test]
    fn a_rate_and_a_recipient_are_inseparable() {
        // The pairing that previously signed a fee with an empty recipient, or
        // silently zeroed a configured fee, is now unrepresentable: every
        // resolution yields both or neither.
        for (over, config) in [
            (None, None),
            (None, Some(configured())),
            (Some(overridden()), None),
            (Some(overridden()), Some(configured())),
        ] {
            let (fee, pubkey) = resolve_integrator(over.as_ref(), config.as_ref()).unwrap();
            assert_eq!(
                fee > 0,
                pubkey.is_some(),
                "fee {fee} paired with recipient {pubkey:?}"
            );
        }
    }

    #[test]
    fn a_fee_above_the_principal_is_rejected_from_either_source() {
        let mut absurd = configured();
        absurd.fee_bps = MAX_INTEGRATOR_FEE_BPS + 1;

        assert!(resolve_integrator(None, Some(&absurd)).is_err());
        assert!(resolve_integrator(Some(&absurd), None).is_err());
        assert!(resolve_integrator(Some(&absurd), Some(&configured())).is_err());
        // The configured one is not consulted when an override is present.
        assert!(resolve_integrator(Some(&configured()), Some(&absurd)).is_ok());
    }
}
