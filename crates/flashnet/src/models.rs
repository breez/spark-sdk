use std::str::FromStr;

use serde::Deserializer;
use serde::{Deserialize, Serialize};
use serde_with::DisplayFromStr;
use serde_with::serde_as;
use spark::Network;
use spark_wallet::PublicKey;

use crate::utils::decode_token_identifier;
use crate::{BTC_ASSET_ADDRESS, FlashnetError};

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

#[derive(Debug, Clone)]
pub struct GetMinAmountsRequest {
    pub asset_in_address: String,
    pub asset_out_address: String,
}

impl GetMinAmountsRequest {
    pub(crate) fn decode_token_identifiers(&self, network: Network) -> Result<Self, FlashnetError> {
        Ok(Self {
            asset_in_address: decode_token_identifier(&self.asset_in_address, network)?,
            asset_out_address: decode_token_identifier(&self.asset_out_address, network)?,
        })
    }
}

#[derive(Default)]
pub struct GetMinAmountsResponse {
    pub asset_in_min: Option<u128>,
    pub asset_out_min: Option<u128>,
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

impl Pool {
    /// Calculate the required amount of the input asset to receive the desired amount of the output asset,
    /// taking into account pool reserves, fees, and slippage. Returns an error if the calculation
    /// cannot be performed due to insufficient pool data or invalid parameters.
    ///
    /// If calculating the required input amount when the output asset is BTC, the output amount
    /// is rounded up to the next multiple of 64 sats to account for BTC variable fee bit masking.
    pub fn calculate_amount_in(
        &self,
        asset_in_address: &str,
        amount_out: u128,
        max_slippage_bps: u32,
        network: Network,
    ) -> Result<u128, FlashnetError> {
        let asset_in_address = decode_token_identifier(asset_in_address, network)?;
        let is_a_to_b = asset_in_address == self.asset_a_address;

        // Round up to next multiple of 64 (2^6) to account for BTC variable fee bit masking
        let asset_out_address = if is_a_to_b {
            &self.asset_b_address
        } else {
            &self.asset_a_address
        };
        let amount_out = if asset_out_address == BTC_ASSET_ADDRESS {
            amount_out.saturating_add(63) & !63
        } else {
            amount_out
        };

        // Add slippage buffer to amount_out first
        // amount_out_with_slippage = amount_out * (max_slippage_bps + 10_000) / 10_000
        let amount_out_with_slippage = amount_out
            .saturating_mul(u128::from(max_slippage_bps).saturating_add(10_000))
            .saturating_div(10_000);

        // Account for fees on output (only for A to B swaps)
        // amount_out_effective = amount_out × (10_000 + fee_bps) / 10_000
        let amount_out_before_output_fees = if is_a_to_b {
            let output_fee_bps = self.host_fee_bps;
            amount_out_with_slippage
                .saturating_mul(u128::from(output_fee_bps).saturating_add(10_000))
                .saturating_div(10_000)
        } else {
            amount_out_with_slippage
        };

        // Helper function for ceiling division: (numerator + denominator - 1) / denominator
        #[allow(clippy::arithmetic_side_effects)]
        let div_ceil = |numerator: u128, denominator: u128| -> u128 {
            numerator
                .saturating_add(denominator.saturating_sub(1))
                .saturating_div(denominator)
        };

        // Calculate amount_in before input fees
        let amount_in_before_input_fees = if let (Some(reserve_a), Some(reserve_b)) =
            (self.asset_a_reserve, self.asset_b_reserve)
        {
            // Calculate amount_in using reserves with integer arithmetic
            // amount_in = (reserve_in × amount_out) / (reserve_out - amount_out)
            let (reserve_in, reserve_out) = if is_a_to_b {
                (reserve_a, reserve_b)
            } else {
                (reserve_b, reserve_a)
            };

            // Check for overflow/underflow conditions
            if amount_out_before_output_fees >= reserve_out {
                return Err(FlashnetError::Generic(
                    "Amount out exceeds reserve out".to_string(),
                ));
            }

            let numerator = reserve_in.saturating_mul(amount_out_before_output_fees);
            let denominator = reserve_out.saturating_sub(amount_out_before_output_fees);

            div_ceil(numerator, denominator)
        } else if let Some(current_price_a_in_b) = self.current_price_a_in_b {
            // Convert floating point price to fixed-point integer representation
            // Use adaptive scaling to handle both small and large price ratios
            const PRICE_SCALE: u128 = 1_000_000_000;
            const LARGE_PRICE_SCALE: u128 = 1_000_000;
            const LARGE_PRICE_THRESHOLD: f64 = 100.0;

            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let (numerator, denominator) = if is_a_to_b {
                // A to B: multiply by price (works well with fixed-point)
                let price_scaled = (current_price_a_in_b * PRICE_SCALE as f64) as u128;
                (
                    amount_out_before_output_fees.saturating_mul(price_scaled),
                    PRICE_SCALE,
                )
            } else if current_price_a_in_b > LARGE_PRICE_THRESHOLD {
                // B to A with large price: use direct integer division for better precision
                let price_scaled = (current_price_a_in_b * LARGE_PRICE_SCALE as f64) as u128;
                (
                    amount_out_before_output_fees.saturating_mul(LARGE_PRICE_SCALE),
                    price_scaled,
                )
            } else {
                // B to A with normal/small price: use scaled inverse
                let price_scaled = (PRICE_SCALE as f64 / current_price_a_in_b) as u128;
                (
                    amount_out_before_output_fees.saturating_mul(price_scaled),
                    PRICE_SCALE,
                )
            };

            div_ceil(numerator, denominator)
        } else {
            return Err(FlashnetError::Generic(
                "Insufficient pool data to calculate amount_in".to_string(),
            ));
        };

        // Account for fees on input
        // amount_in = amount_in_before_input_fees × (10_000 + fee_bps) / 10_000
        let input_fee_bps = if is_a_to_b {
            // A to B: only LP fees on input
            self.lp_fee_bps
        } else {
            // B to A: both host and LP fees on input
            self.host_fee_bps.saturating_add(self.lp_fee_bps)
        };

        let amount_in = amount_in_before_input_fees
            .saturating_mul(u128::from(input_fee_bps).saturating_add(10_000))
            .saturating_div(10_000);

        Ok(amount_in)
    }
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

    fn create_default_test_pool() -> Pool {
        create_test_pool(
            0,
            20,
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            Some(155_123_677),
            Some(143_108_978_165),
            Some(922.547_613_635_651_9),
        )
    }

    fn create_test_pool(
        host_fee_bps: u32,
        lp_fee_bps: u32,
        a_address: &str,
        b_address: &str,
        a_reserve: Option<u128>,
        b_reserve: Option<u128>,
        current_price_a_in_b: Option<f64>,
    ) -> Pool {
        Pool {
            lp_public_key: PublicKey::from_str(
                "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            )
            .unwrap(),
            host_name: "flashnet".to_string(),
            host_fee_bps,
            lp_fee_bps,
            asset_a_address: a_address.to_string(),
            asset_b_address: b_address.to_string(),
            asset_a_reserve: a_reserve,
            asset_b_reserve: b_reserve,
            virtual_reserve_a: None,
            virtual_reserve_b: None,
            threshold_pct: None,
            current_price_a_in_b,
            tvl_asset_b: Some(316_135_065_311),
            volume_24h_asset_b: Some(7_559_202_607),
            price_change_percent_24h: Some(2.74),
            curve_type: Some(CurveType::ConstantProduct),
            initial_reserve_a: None,
            bonding_progress_percent: None,
            graduation_threshold_amount: None,
            created_at: "2025-09-22 19:09:36.661269 +00:00:00".to_string(),
            updated_at: "2025-12-03 12:43:53.903531 +00:00:00".to_string(),
        }
    }

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

    #[test]
    fn test_calculate_amount_in_a_to_b_with_reserves() {
        // A to B swap: LP fees on input, host fees on output
        let pool = create_test_pool(
            100, // 1% host fee
            20,  // 0.2% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            Some(1_000_000_000),  // 1B asset A reserve
            Some(10_000_000_000), // 10B asset B reserve
            None,
        );

        let amount_out = 1_000_000; // Want 1M of asset B
        let max_slippage_bps = 100; // 1% slippage

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Expected calculation:
        // amount_out_with_slippage = 1_000_000 * (100 + 10_000) / 10_000 = 1_010_000
        // output_fee_rate = 100 / 10_000 = 0.01
        // amount_out_before_output_fees = 1_010_000 * (1 + 0.01) = 1_020_100
        // amount_in_before_input_fees = (1_000_000_000 * 1_020_100) / (10_000_000_000 - 1_020_100) ≈ 101_031.03
        // input_fee_rate = 20 / 10_000 = 0.002
        // amount_in = 101_031.03 * (1 + 0.002) ≈ 102_233.24
        assert!(amount_in > 102_000 && amount_in < 103_000);
    }
    #[test]
    fn test_calculate_amount_in_b_to_a_with_reserves() {
        // B to A swap: both host and LP fees on input
        let pool = create_test_pool(
            100, // 1% host fee
            20,  // 0.2% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            Some(1_000_000_000),  // 1B asset A reserve
            Some(10_000_000_000), // 10B asset B reserve
            None,
        );

        let amount_out = 100_000; // Want 100K of asset A
        let max_slippage_bps = 100; // 1% slippage

        let result = pool.calculate_amount_in(
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Expected calculation:
        // amount_out_with_slippage = 100_000 * (100 + 10_000) / 10_000 = 101_000
        // No output fees for B to A
        // amount_in_before_input_fees = (10_000_000_000 * 101_000) / (1_000_000_000 - 101_000) ≈ 1_010_101.01
        // input_fee_rate = (100 + 20) / 10_000 = 0.012
        // amount_in = 1_010_101.01 * (1 + 0.012) ≈ 1_022_222.22
        assert!(amount_in > 1_020_000 && amount_in < 1_025_000);
    }

    #[test]
    fn test_calculate_amount_in_with_price_only() {
        // Test using price when reserves are not available (A to B)
        let pool = create_test_pool(
            100, // 1% host fee
            20,  // 0.2% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            None,       // No reserve A
            None,       // No reserve B
            Some(10.0), // Price: 1 A = 10 B
        );

        let amount_out = 1_000_000; // Want 1M of asset B
        let max_slippage_bps = 100; // 1% slippage

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Expected calculation:
        // amount_out_with_slippage = 1_000_000 * 1.01 = 1_010_000
        // amount_out_before_output_fees = 1_010_000 * 1.01 = 1_020_100
        // amount_in_before_input_fees = 1_020_100 * 10 = 10_201_000
        // amount_in = 10_201_000 * 1.002 = 10_221_402
        assert!(amount_in > 10_220_000 && amount_in < 10_222_000);
    }

    #[test]
    fn test_calculate_amount_in_with_price_only_b_to_a() {
        // Test using price when reserves are not available (B to A)
        let pool = create_test_pool(
            100, // 1% host fee
            20,  // 0.2% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            None,       // No reserve A
            None,       // No reserve B
            Some(10.0), // Price: 1 A = 10 B
        );

        let amount_out = 100_000; // Want 100K of asset A
        let max_slippage_bps = 100; // 1% slippage

        let result = pool.calculate_amount_in(
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Expected calculation with fixed-point math (PRICE_SCALE = 10^9):
        // amount_out_with_slippage = 100_000 * 1.01 = 101_000
        // No output fees for B to A
        // Inverse price scaled: (10^9 / 10.0) = 100_000_000
        // amount_in_before_input_fees = (101_000 * 100_000_000 + 999_999_999) / 1_000_000_000 = 10_100
        // input_fee_rate = (100 + 20) / 10_000 = 0.012
        // amount_in = 10_100 * 1.012 = 10_221
        assert!(amount_in > 10_220 && amount_in < 10_230);
    }

    #[test]
    fn test_calculate_amount_in_with_realistic_btc_usd_price() {
        // Test with realistic BTC/USD-like price (B to A swap with high price)
        let pool = create_test_pool(
            100, // 1% host fee
            20,  // 0.2% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            None,         // No reserve A
            None,         // No reserve B
            Some(1000.0), // Price: 1 A (BTC) = 1000 B (USD-like token)
        );

        let amount_out = 100_000; // Want 100K sats of BTC
        let max_slippage_bps = 100; // 1% slippage

        let result = pool.calculate_amount_in(
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Expected calculation with improved precision for large price ratios:
        // amount_out_with_slippage = 100_000 * 1.01 = 101_000
        // No output fees for B to A
        // Direct division: 101_000 / 1000 = 101
        // input_fee_rate = (100 + 20) / 10_000 = 0.012
        // amount_in = 101 * 1.012 ≈ 102.2
        println!("BTC/USD test - amount_in: {amount_in} (for {amount_out} sats out)",);

        // With improved precision handling, we get accurate results
        assert!((102..=103).contains(&amount_in));
    }

    #[test]
    fn test_calculate_amount_in_with_small_price_reversed() {
        // Test with small price (reversed asset order: A to B with price 0.001)
        // This tests the A→B multiplication path with a small price
        let pool = create_test_pool(
            100,                                                                  // 1% host fee
            20,                                                                   // 0.2% LP fee
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca", // USD-like as asset_a
            "020202020202020202020202020202020202020202020202020202020202020202", // BTC as asset_b
            None,                                                               // No reserve A
            None,                                                               // No reserve B
            Some(0.001), // Price: 1 USD = 0.001 BTC (or 1 BTC = 1000 USD)
        );

        let amount_out = 100_000; // Want 100K sats of BTC (asset_b)
        let max_slippage_bps = 100; // 1% slippage

        // Swapping A→B (USD → BTC)
        let result = pool.calculate_amount_in(
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Expected calculation with A→B and price 0.001:
        // amount_out_with_slippage = 100_000 * 1.01 = 101_000
        // output_fee_bps = 100 (host fee applies to A→B)
        // amount_out_before_output_fees = 101_000 * 1.01 = 102,010
        // price_scaled = 0.001 * 10^9 = 1_000_000
        // amount_in_before_input_fees = (102_010 * 1_000_000 + 999_999_999) / 10^9 ≈ 103
        // input_fee_rate = 20 / 10_000 = 0.002 (only LP fee for A→B)
        // amount_in = 103 * 1.002 ≈ 103.2
        println!("Small price reversed - amount_in: {amount_in} (for {amount_out} sats out)",);

        // With small price and A→B direction, we get a small amount_in
        assert!((103..=104).contains(&amount_in));
    }

    #[test]
    fn test_calculate_amount_in_zero_slippage() {
        let pool = create_test_pool(
            0,  // 0% host fee
            20, // 0.2% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            Some(1_000_000_000),
            Some(10_000_000_000),
            None,
        );

        let amount_out = 1_000_000;
        let max_slippage_bps = 0; // 0% slippage

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // With 0 slippage, should be base calculation
        assert!(amount_in > 100_000 && amount_in < 101_000);
    }

    #[test]
    fn test_calculate_amount_in_high_slippage() {
        let pool = create_test_pool(
            100,
            20,
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            Some(1_000_000_000),
            Some(10_000_000_000),
            None,
        );

        let amount_out = 1_000_000;
        let max_slippage_bps = 1000; // 10% slippage

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Higher slippage means more buffer needed
        assert!(amount_in > 110_000);
    }

    #[test]
    fn test_calculate_amount_in_no_fees() {
        let pool = create_test_pool(
            0, // 0% host fee
            0, // 0% LP fee
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            Some(1_000_000_000),
            Some(10_000_000_000),
            None,
        );

        let amount_out = 1_000_000;
        let max_slippage_bps = 100;

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // With no fees, calculation should be simpler
        // amount_out_with_slippage = 1_010_000
        // amount_in_before_input_fees ≈ 101_010.10
        // No fees applied
        assert!(amount_in > 101_000 && amount_in < 102_000);
    }

    #[test]
    fn test_calculate_amount_in_error_no_data() {
        let pool = create_test_pool(
            100,
            20,
            "020202020202020202020202020202020202020202020202020202020202020202",
            "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
            None, // No reserves
            None,
            None, // No price
        );

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            1_000_000,
            100,
            Network::Regtest,
        );

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FlashnetError::Generic(_)));
    }

    #[test]
    fn test_calculate_amount_in_real_world_values() {
        // Test with realistic pool values
        let pool = create_default_test_pool();

        let amount_out = 10_000; // Want 10K sats
        let max_slippage_bps = 500; // 5% slippage

        let result = pool.calculate_amount_in(
            "020202020202020202020202020202020202020202020202020202020202020202",
            amount_out,
            max_slippage_bps,
            Network::Regtest,
        );

        assert!(result.is_ok());
        let amount_in = result.unwrap();

        // Should return a reasonable amount
        assert!(amount_in > 0);
    }
}
