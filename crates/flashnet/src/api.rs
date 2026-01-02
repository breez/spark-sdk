use std::{collections::HashMap, sync::Arc};

use bitcoin::hashes::{Hash, sha256};
use spark::bech32m_encode_token_id;
use spark_wallet::{PublicKey, SparkAddress, SparkWallet, TransferId, TransferTokenOutput};
use tracing::debug;

use crate::utils::generate_nonce;
use crate::{
    ClawbackIntent, ClawbackRequest, ClawbackResponse, ExecuteSwapIntent, ExecuteSwapResponse,
    GetMinAmountsRequest, GetMinAmountsResponse, ListUserSwapsRequest, ListUserSwapsResponse,
    SignedClawbackRequest, SignedExecuteSwapResponse,
};
use crate::{
    ExecuteSwapRequest, FeatureName, FeatureStatus, FlashnetError, MinAmount, PingResponse,
    SignedExecuteSwapRequest,
    cache::CacheStore,
    config::FlashnetConfig,
    models::{ListPoolsRequest, ListPoolsResponse, SimulateSwapRequest, SimulateSwapResponse},
};

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

pub struct FlashnetClient {
    pub(crate) config: FlashnetConfig,
    pub(crate) cache_store: Arc<CacheStore>,
    pub(crate) spark_wallet: Arc<SparkWallet>,
    pub(crate) http_client: platform_utils::DefaultHttpClient,
}

impl FlashnetClient {
    pub fn new(
        config: FlashnetConfig,
        spark_wallet: Arc<SparkWallet>,
        cache_store: Arc<CacheStore>,
    ) -> Self {
        Self {
            config,
            cache_store,
            spark_wallet,
            http_client: platform_utils::DefaultHttpClient::default(),
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

    pub async fn execute_swap(
        &self,
        request: ExecuteSwapRequest,
    ) -> Result<ExecuteSwapResponse, FlashnetError> {
        let request = request.decode_token_identifiers(self.config.network)?;
        debug!("Execute swap request: {request:?}");

        // Pre-checks
        self.ensure_ping_ok().await?;
        self.ensure_feature_status(FeatureName::AllowSwaps).await?;
        self.ensure_min_amounts(
            &request.asset_in_address,
            &request.asset_out_address,
            request.amount_in,
            Some(request.min_amount_out),
        )
        .await?;

        // Transfer the asset in to the pool
        let transaction_identifier = self
            .transfer_asset(
                request.amount_in,
                &request.asset_in_address,
                &request.pool_id,
                None,
            )
            .await?;

        // Sign and send the execute swap request
        let swap_response_res = self
            .sign_execute_swap(request, &transaction_identifier)
            .await;
        match swap_response_res {
            Ok(response) => Ok(ExecuteSwapResponse::from_signed_execute_swap_response(
                response,
                transaction_identifier,
            )),
            Err(e) => Err(FlashnetError::execution(e, Some(transaction_identifier))),
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
        request: ExecuteSwapRequest,
        asset_in_spark_transfer_id: &str,
    ) -> Result<SignedExecuteSwapResponse, FlashnetError> {
        let nonce = hex::encode(generate_nonce());

        // Auto-populate integrator config from FlashnetConfig if not specified in request
        let (total_integrator_fee_rate_bps, integrator_public_key) = match (
            request.integrator_fee_rate_bps,
            request.integrator_public_key,
        ) {
            (Some(fee), pk) => (fee, pk),
            (None, Some(pk)) => (0, Some(pk)),
            (None, None) => {
                if let Some(config) = &self.config.integrator_config {
                    (config.fee_bps, Some(config.pubkey))
                } else {
                    (0, None)
                }
            }
        };

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
            asset_in_address: request.asset_in_address,
            asset_out_address: request.asset_out_address,
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
    ) -> Result<String, FlashnetError> {
        let receiver_address = SparkAddress::new(*receiver_public_key, self.config.network, None);
        let id = if asset_address == BTC_ASSET_ADDRESS {
            // Send a spark transfer
            self.spark_wallet
                .transfer(
                    u64::try_from(amount).map_err(|e| {
                        FlashnetError::Generic(format!("Failed to convert amount to u64: {e}"))
                    })?,
                    &receiver_address,
                    transfer_id,
                )
                .await?
                .id
                .to_string()
        } else {
            // Send a token transfer
            let asset_address_hex = hex::decode(asset_address).map_err(|e| {
                FlashnetError::Generic(format!("Failed to decode asset address from hex: {e}"))
            })?;
            let token_id = bech32m_encode_token_id(&asset_address_hex, self.config.network)
                .map_err(|e| FlashnetError::Generic(format!("Failed to encode token id: {e}")))?;
            self.spark_wallet
                .transfer_tokens(
                    vec![TransferTokenOutput {
                        token_id,
                        amount,
                        receiver_address,
                        spark_invoice: None,
                    }],
                    None,
                    None,
                )
                .await?
                .hash
        };
        Ok(id)
    }
}
