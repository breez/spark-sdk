use std::str::FromStr;

use serde::Deserializer;
use serde::{Deserialize, Serialize};
use serde_with::DisplayFromStr;
use serde_with::serde_as;
use spark::Network;
use spark_wallet::PublicKey;

use crate::FlashnetError;
use crate::utils::decode_token_identifier;

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChallengeRequest {
    pub public_key: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChallengeResponse {
    pub challenge: String,
    pub challenge_string: String,
    pub request_id: String,
}

#[derive(Debug, Clone)]
pub struct ClawbackRequest {
    pub pool_id: PublicKey,
    pub transfer_id: String,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClawbackIntent {
    pub(crate) sender_public_key: PublicKey,
    pub(crate) spark_transfer_id: String,
    pub(crate) lp_identity_public_key: PublicKey,
    pub(crate) nonce: String,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignedClawbackRequest {
    pub(crate) sender_public_key: PublicKey,
    pub(crate) spark_transfer_id: String,
    pub(crate) lp_identity_public_key: PublicKey,
    pub(crate) nonce: String,
    pub(crate) signature: String,
}

#[serde_as]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ClawbackResponse {
    pub request_id: String,
    pub accepted: bool,
    pub internal_request_id: String,
    pub spark_status_tracking_id: String,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CurveType {
    ConstantProduct,
    SingleSided,
}

impl std::fmt::Display for CurveType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CurveType::ConstantProduct => write!(f, "CONSTANT_PRODUCT"),
            CurveType::SingleSided => write!(f, "SINGLE_SIDED"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecuteSwapRequest {
    pub pool_id: PublicKey,
    pub asset_in_address: String,
    pub asset_out_address: String,
    pub amount_in: u128,
    pub max_slippage_bps: u32,
    pub min_amount_out: u128,
    pub integrator_fee_rate_bps: Option<u32>,
    pub integrator_public_key: Option<PublicKey>,
}

impl ExecuteSwapRequest {
    pub(crate) fn decode_token_identifiers(&self, network: Network) -> Result<Self, FlashnetError> {
        Ok(Self {
            asset_in_address: decode_token_identifier(&self.asset_in_address, network)?,
            asset_out_address: decode_token_identifier(&self.asset_out_address, network)?,
            ..self.clone()
        })
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteSwapIntent {
    pub(crate) user_public_key: PublicKey,
    pub(crate) lp_identity_public_key: PublicKey,
    pub(crate) asset_in_spark_transfer_id: String,
    pub(crate) asset_in_address: String,
    pub(crate) asset_out_address: String,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) amount_in: u128,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) min_amount_out: u128,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) max_slippage_bps: u32,
    pub(crate) nonce: String,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) total_integrator_fee_rate_bps: u32,
}

#[serde_as]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteSwapResponse {
    pub transfer_id: String,
    pub request_id: String,
    pub accepted: bool,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub amount_out: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub fee_amount: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub execution_price: Option<f64>,
    pub asset_out_address: Option<String>,
    pub asset_in_address: Option<String>,
    pub outbound_transfer_id: Option<String>,
    pub error: Option<String>,
    pub refunded_asset_address: Option<String>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub refunded_amount: Option<u128>,
    pub refund_transfer_id: Option<String>,
}

impl ExecuteSwapResponse {
    pub(crate) fn from_signed_execute_swap_response(
        response: SignedExecuteSwapResponse,
        transfer_id: String,
    ) -> Self {
        Self {
            transfer_id,
            request_id: response.request_id,
            accepted: response.accepted,
            amount_out: response.amount_out,
            fee_amount: response.fee_amount,
            execution_price: response.execution_price,
            asset_out_address: response.asset_out_address,
            asset_in_address: response.asset_in_address,
            outbound_transfer_id: response.outbound_transfer_id,
            error: response.error,
            refunded_asset_address: response.refunded_asset_address,
            refunded_amount: response.refunded_amount,
            refund_transfer_id: response.refund_transfer_id,
        }
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignedExecuteSwapRequest {
    pub(crate) user_public_key: PublicKey,
    pub(crate) pool_id: PublicKey,
    pub(crate) asset_in_address: String,
    pub(crate) asset_out_address: String,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) amount_in: u128,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) max_slippage_bps: u32,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) min_amount_out: u128,
    pub(crate) asset_in_spark_transfer_id: String,
    pub(crate) nonce: String,
    #[serde_as(as = "DisplayFromStr")]
    pub(crate) total_integrator_fee_rate_bps: u32,
    pub(crate) integrator_public_key: String,
    pub(crate) signature: String,
}

#[serde_as]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignedExecuteSwapResponse {
    pub(crate) request_id: String,
    pub(crate) accepted: bool,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub(crate) amount_out: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub fee_amount: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub(crate) execution_price: Option<f64>,
    pub(crate) asset_out_address: Option<String>,
    pub(crate) asset_in_address: Option<String>,
    pub(crate) outbound_transfer_id: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) refunded_asset_address: Option<String>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub(crate) refunded_amount: Option<u128>,
    pub(crate) refund_transfer_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FeatureName {
    MasterKillSwitch,
    AllowWithdrawFees,
    AllowPoolCreation,
    AllowSwaps,
    AllowAddLiquidity,
    AllowRouteSwaps,
    AllowWithdrawLiquidity,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) struct FeatureStatus {
    pub feature_name: FeatureName,
    pub enabled: bool,
    pub reason: Option<String>,
}

#[serde_as]
#[derive(Serialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListPoolsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_a_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_b_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_volume_24h: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_tvl: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<Vec<DisplayFromStr>>")]
    pub curve_types: Option<Vec<CurveType>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub sort: Option<PoolSortOrder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_updated_at: Option<String>,
}

impl ListPoolsRequest {
    pub(crate) fn decode_token_identifiers(&self, network: Network) -> Result<Self, FlashnetError> {
        Ok(Self {
            asset_a_address: self
                .asset_a_address
                .as_ref()
                .map(|addr| decode_token_identifier(addr, network))
                .transpose()?,
            asset_b_address: self
                .asset_b_address
                .as_ref()
                .map(|addr| decode_token_identifier(addr, network))
                .transpose()?,
            ..self.clone()
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListPoolsResponse {
    pub pools: Vec<Pool>,
    pub total_count: u32,
}

#[serde_as]
#[derive(Serialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListUserSwapsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_lp_pubkey: Option<PublicKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_in_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_out_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_amount_in: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_amount_in: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub sort: Option<SwapSortOrder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

impl ListUserSwapsRequest {
    pub(crate) fn decode_token_identifiers(&self, network: Network) -> Result<Self, FlashnetError> {
        Ok(Self {
            asset_in_address: self
                .asset_in_address
                .as_ref()
                .map(|addr| decode_token_identifier(addr, network))
                .transpose()?,
            asset_out_address: self
                .asset_out_address
                .as_ref()
                .map(|addr| decode_token_identifier(addr, network))
                .transpose()?,
            ..self.clone()
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ListUserSwapsResponse {
    pub swaps: Vec<Swap>,
    pub total_count: u32,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) struct MinAmount {
    pub asset_identifier: String,
    #[serde(deserialize_with = "deserialize_string_or_u128")]
    pub min_amount: u128,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PingResponse {
    pub status: String,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Pool {
    #[serde_as(as = "DisplayFromStr")]
    pub lp_public_key: PublicKey,
    pub host_name: String,
    pub host_fee_bps: u32,
    pub lp_fee_bps: u32,
    pub asset_a_address: String,
    pub asset_b_address: String,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub asset_a_reserve: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub asset_b_reserve: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub virtual_reserve_a: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub virtual_reserve_b: Option<u128>,
    pub threshold_pct: Option<u8>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub current_price_a_in_b: Option<f64>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub tvl_asset_b: Option<u64>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub volume_24h_asset_b: Option<u64>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub price_change_percent_24h: Option<f64>,
    pub curve_type: Option<CurveType>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub initial_reserve_a: Option<u128>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub bonding_progress_percent: Option<f64>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub graduation_threshold_amount: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PoolSortOrder {
    CreatedAtAsc,
    CreatedAtDesc,
    Volume24hAsc,
    Volume24hDesc,
    TvlAsc,
    TvlDesc,
}

impl std::fmt::Display for PoolSortOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolSortOrder::CreatedAtAsc => write!(f, "CREATED_AT_ASC"),
            PoolSortOrder::CreatedAtDesc => write!(f, "CREATED_AT_DESC"),
            PoolSortOrder::Volume24hAsc => write!(f, "VOLUME24H_ASC"),
            PoolSortOrder::Volume24hDesc => write!(f, "VOLUME24H_DESC"),
            PoolSortOrder::TvlAsc => write!(f, "TVL_ASC"),
            PoolSortOrder::TvlDesc => write!(f, "TVL_DESC"),
        }
    }
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SimulateSwapRequest {
    pub pool_id: PublicKey,
    pub asset_in_address: String,
    pub asset_out_address: String,
    #[serde_as(as = "DisplayFromStr")]
    pub amount_in: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrator_bps: Option<u32>,
}

impl SimulateSwapRequest {
    pub(crate) fn decode_token_identifiers(&self, network: Network) -> Result<Self, FlashnetError> {
        Ok(Self {
            asset_in_address: decode_token_identifier(&self.asset_in_address, network)?,
            asset_out_address: decode_token_identifier(&self.asset_out_address, network)?,
            ..self.clone()
        })
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SimulateSwapResponse {
    #[serde_as(as = "DisplayFromStr")]
    pub amount_out: u128,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub execution_price: Option<f64>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub fee_paid_asset_in: Option<u128>,
    pub price_impact_pct: Option<String>,
    pub warning_message: Option<String>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Swap {
    pub id: String,
    #[serde_as(as = "DisplayFromStr")]
    pub pool_lp_public_key: PublicKey,
    #[serde(deserialize_with = "deserialize_string_or_u128")]
    pub amount_in: u128,
    #[serde(deserialize_with = "deserialize_string_or_u128")]
    pub amount_out: u128,
    pub asset_in_address: String,
    pub asset_out_address: String,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub price: Option<f64>,
    pub timestamp: String,
    #[serde(deserialize_with = "deserialize_string_or_u128")]
    pub fee_paid: u128,
    pub pool_asset_a_address: Option<String>,
    pub pool_asset_b_address: Option<String>,
    pub inbound_transfer_id: String,
    pub outbound_transfer_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum SwapSortOrder {
    TimestampDesc,
    TimestampAsc,
    AmountInDesc,
    AmountInAsc,
    AmountOutDesc,
    AmountOutAsc,
}

impl std::fmt::Display for SwapSortOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwapSortOrder::TimestampAsc => write!(f, "timestampAsc"),
            SwapSortOrder::TimestampDesc => write!(f, "timestampDesc"),
            SwapSortOrder::AmountInAsc => write!(f, "amountInAsc"),
            SwapSortOrder::AmountInDesc => write!(f, "amountInDesc"),
            SwapSortOrder::AmountOutAsc => write!(f, "amountOutAsc"),
            SwapSortOrder::AmountOutDesc => write!(f, "amountOutDesc"),
        }
    }
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyRequest {
    pub public_key: String,
    pub signature: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyResponse {
    pub access_token: String,
}

fn deserialize_string_or_u128<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + std::fmt::Debug,
{
    let value: serde_json::Value = Deserialize::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => num
            .as_u128()
            .map(|n| {
                T::from_str(&n.to_string())
                    .map_err(|_| serde::de::Error::custom("Failed to parse number"))
            })
            .transpose()?
            .ok_or_else(|| serde::de::Error::custom("Invalid number")),
        serde_json::Value::String(s) => {
            T::from_str(&s).map_err(|_| serde::de::Error::custom("Failed to parse string"))
        }
        _ => Err(serde::de::Error::custom("Expected a string or number")),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_execute_swap_intent_serialization() {
        let intent = ExecuteSwapIntent {
            user_public_key: PublicKey::from_str(
                "0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d",
            )
            .unwrap(),
            lp_identity_public_key: PublicKey::from_str(
                "02a1633caf0d6d2a8b3f4e1f5e6d7c8b9a0b1c2d3e4f5061728394a5b6c7d8e9fa",
            )
            .unwrap(),
            asset_in_spark_transfer_id: "transfer123".to_string(),
            asset_in_address: "03b06b7c3e39bf922be19b7ad5f19554bb7991cae585ed2e3374d51213ff4eeb3c"
                .to_string(),
            asset_out_address: "020202020202020202020202020202020202020202020202020202020202020202"
                .to_string(),
            amount_in: 1_000_000,
            min_amount_out: 950_000,
            max_slippage_bps: 500,
            nonce: "nonce123".to_string(),
            total_integrator_fee_rate_bps: 50,
        };

        let serialized = serde_json::to_string(&intent).unwrap();
        let expected_json = r#"{"userPublicKey":"0315299b3f9f4e2beb8576ea2bf72ea1bc741eb255bfc3f6387de4d47b5c05972d","lpIdentityPublicKey":"02a1633caf0d6d2a8b3f4e1f5e6d7c8b9a0b1c2d3e4f5061728394a5b6c7d8e9fa","assetInSparkTransferId":"transfer123","assetInAddress":"03b06b7c3e39bf922be19b7ad5f19554bb7991cae585ed2e3374d51213ff4eeb3c","assetOutAddress":"020202020202020202020202020202020202020202020202020202020202020202","amountIn":"1000000","minAmountOut":"950000","maxSlippageBps":"500","nonce":"nonce123","totalIntegratorFeeRateBps":"50"}"#;
        assert_eq!(serialized, expected_json);
    }

    #[test]
    fn test_deserialize_string_or_u128() {
        let json = r#"
        {
            "asset_identifier": "03b06b7c3e39bf922be19b7ad5f19554bb7991cae585ed2e3374d51213ff4eeb3c",
            "min_amount": "1000000",
            "enabled": true
        }
        "#;
        let parsed: MinAmount = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.min_amount, 1_000_000_u128);

        let json2 = r#"
        {
            "asset_identifier": "020202020202020202020202020202020202020202020202020202020202020202",
            "min_amount": 500000,
            "enabled": false
        }
        "#;
        let parsed2: MinAmount = serde_json::from_str(json2).unwrap();
        assert_eq!(parsed2.min_amount, 500_000_u128);
    }
}
