//! Request/response types for the Flashnet Orchestra API.
//!
//! Mirrors the machine-readable schema the API serves at
//! <https://orchestration.flashnet.xyz/openapi.json> (rendered at `/docs`),
//! which is authoritative where the narrative docs at
//! <https://docs.flashnet.xyz/products/orchestration/api/quotes-and-orders>
//! are incomplete.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub source_chain: String,
    pub source_asset: String,
    pub destination_chain: String,
    pub destination_asset: String,
    #[serde(default)]
    pub exact_out_eligible: bool,
    pub source: RouteAsset,
    pub destination: RouteAsset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteAsset {
    pub chain: String,
    pub asset: String,
    pub contract_address: Option<String>,
    pub decimals: u8,
    pub chain_id: Option<String>,
}

// ---------------------------------------------------------------------------
// GET /v1/orchestration/limits
// ---------------------------------------------------------------------------

/// `/limits` returns the same route set as `/routes` under the same filters,
/// with a `limits` block attached to each entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsResponse {
    pub routes: Vec<RouteWithLimits>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteWithLimits {
    #[serde(flatten)]
    pub route: Route,
    pub limits: RouteLimits,
}

/// Only the bounds the SDK surfaces are modelled. The `constraints` array and
/// the `fiatUsd` / `exactOut` / `dynamicProviderLimits` groups are ignored on
/// deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteLimits {
    #[serde(default)]
    pub order_notional_usd: Option<UsdBounds>,
    #[serde(default)]
    pub exact_in: Option<ModeLimits>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsdBounds {
    #[serde(default)]
    pub min_cents: Option<String>,
    #[serde(default)]
    pub max_cents: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeLimits {
    #[serde(default)]
    pub supported: bool,
    #[serde(default)]
    pub request_amount: Option<RequestAmountLimits>,
}

/// Bounds on the amount the caller passes to `/estimate` and `/quote`.
///
/// `*_amount_smallest` is denominated in the request leg's base units (the
/// source leg for `exact_in`); `*_usd_cents` in USD cents. Either group can be
/// absent, and the two are independent: a route can publish a base-unit dust
/// floor, a USD notional floor, both, or neither.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestAmountLimits {
    #[serde(default)]
    pub min_amount_smallest: Option<String>,
    #[serde(default)]
    pub max_amount_smallest: Option<String>,
    #[serde(default)]
    pub min_usd_cents: Option<String>,
    #[serde(default)]
    pub max_usd_cents: Option<String>,
}

// ---------------------------------------------------------------------------
// GET /v1/orchestration/estimate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateRequest {
    pub source_chain: String,
    pub source_asset: String,
    pub destination_chain: String,
    pub destination_asset: String,
    pub amount: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_mode: Option<AmountMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affiliate_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimateResponse {
    #[serde(default)]
    pub amount_mode: Option<AmountMode>,
    pub estimated_out: String,
    pub fee_amount: String,
    #[serde(default)]
    pub rounding_fee_amount: Option<String>,
    pub fee_bps: u32,
    pub total_fee_amount: String,
    #[serde(default)]
    pub app_fee_amount: Option<String>,
    #[serde(default)]
    pub app_fee_platform_cut_amount: Option<String>,
    #[serde(default)]
    pub app_fees: Vec<AppFeeResult>,
    pub fee_asset: String,
    #[serde(default)]
    pub fee_asset_details: Option<FeeAssetDetails>,
    #[serde(default)]
    pub route: Vec<String>,
}

/// Chain and precision of a response's `feeAsset`. Every fee amount is in
/// this asset's smallest units, which can differ from both route sides.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeAssetDetails {
    #[serde(default)]
    pub chain: Option<String>,
    #[serde(default)]
    pub asset: Option<String>,
    pub decimals: u32,
    #[serde(default)]
    pub contract_address: Option<String>,
    #[serde(default)]
    pub chain_id: Option<String>,
}

// ---------------------------------------------------------------------------
// POST /v1/orchestration/quote
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AmountMode {
    ExactIn,
    ExactOut,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppFeeRequest {
    pub recipient: String,
    /// Fee in basis points (1..10000).
    pub fee: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub source_chain: String,
    pub source_asset: String,
    pub destination_chain: String,
    pub destination_asset: String,
    pub amount: String,
    pub recipient_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_mode: Option<AmountMode>,
    /// Required for `exact_out` quotes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slippage_bps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zeroconf_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub app_fees: Vec<AppFeeRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affiliate_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppFeeResult {
    #[serde(default)]
    pub affiliate_id: Option<String>,
    pub recipient: String,
    pub fee_bps: u32,
    pub amount: String,
    #[serde(default)]
    pub platform_cut_amount: Option<String>,
    #[serde(default)]
    pub recipient_amount: Option<String>,
}

/// A quote for a cross-chain send, and the address to deposit against it.
///
/// The `exact_out` and price-lock fields the API also returns are not modelled.
/// Quotes are requested as `exact_in`, which carries the integrator fee that
/// `exact_out` does not, and no price lock is asked for, so the server leaves
/// both groups unset. Unmodelled fields are ignored on deserialization.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteResponse {
    pub quote_id: String,
    pub deposit_address: String,
    pub amount_in: String,
    pub estimated_out: String,
    pub fee_amount: String,
    #[serde(default)]
    pub rounding_fee_amount: Option<String>,
    #[serde(default)]
    pub sweep_fee_amount: Option<String>,
    #[serde(default)]
    pub network_cost_amount: Option<String>,
    pub total_fee_amount: String,
    #[serde(default)]
    pub app_fee_amount: Option<String>,
    #[serde(default)]
    pub app_fee_platform_cut_amount: Option<String>,
    #[serde(default)]
    pub app_fees: Vec<AppFeeResult>,
    pub fee_asset: String,
    #[serde(default)]
    pub fee_asset_details: Option<FeeAssetDetails>,
    pub fee_bps: u32,
    #[serde(default)]
    pub route: Vec<String>,
    pub expires_at: String,
    #[serde(default)]
    pub zeroconf_enabled: Option<bool>,
    #[serde(default)]
    pub amount_mode: Option<AmountMode>,
}

// ---------------------------------------------------------------------------
// POST /v1/orchestration/submit
// ---------------------------------------------------------------------------

/// Submit request body. Field shape varies by source chain; we currently only
/// implement the Spark variant since cross-chain sends from Breez always
/// originate on Spark.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitRequest {
    pub quote_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spark_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_spark_address: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResponse {
    pub order_id: String,
    pub status: OrderStatus,
    /// Opaque token that must be presented (via `X-Read-Token` header) when
    /// querying the order status. Binds (partnerId, apiKeyId, orderId).
    #[serde(default)]
    pub read_token: Option<String>,
}

impl std::fmt::Debug for SubmitResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubmitResponse")
            .field("order_id", &self.order_id)
            .field("status", &self.status)
            .field(
                "read_token",
                &self.read_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

// ---------------------------------------------------------------------------
// GET /v1/orchestration/status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Processing,
    Confirming,
    Bridging,
    Swapping,
    AwaitingApproval,
    Refunding,
    Delivering,
    Completed,
    Failed,
    Refunded,
    /// Catch-all for any status variant Orchestra adds in the future.
    #[serde(other)]
    Unknown,
}

impl OrderStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Refunded)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusResponse {
    pub order: Order,
    #[serde(default)]
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub id: String,
    pub status: OrderStatus,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub quote_id: Option<String>,
    #[serde(default)]
    pub source_chain: Option<String>,
    #[serde(default)]
    pub source_asset: Option<String>,
    #[serde(default)]
    pub source_address: Option<String>,
    #[serde(default)]
    pub source_tx_hash: Option<String>,
    #[serde(default)]
    pub source_tx_vout: Option<u32>,
    #[serde(default)]
    pub sweep_tx_hash: Option<String>,
    #[serde(default)]
    pub destination_chain: Option<String>,
    #[serde(default)]
    pub destination_asset: Option<String>,
    #[serde(default)]
    pub destination_address: Option<String>,
    /// Settlement transaction on the destination chain, set once the order is
    /// delivered.
    #[serde(default)]
    pub destination_tx_hash: Option<String>,
    #[serde(default)]
    pub deposit_address: Option<String>,
    #[serde(default)]
    pub recipient_address: Option<String>,
    #[serde(default)]
    pub amount_in: Option<String>,
    #[serde(default)]
    pub amount_out: Option<String>,
    #[serde(default)]
    pub amount_fiat_usd: Option<String>,
    #[serde(default)]
    pub amount_fiat_currency: Option<String>,
    #[serde(default)]
    pub spot_usd_per_btc: Option<String>,
    #[serde(default)]
    pub fee_bps: Option<u32>,
    #[serde(default)]
    pub fee_amount: Option<String>,
    #[serde(default)]
    pub fee_asset: Option<String>,
    #[serde(default)]
    pub rounding_fee_amount: Option<String>,
    #[serde(default)]
    pub slippage_bps: Option<u32>,
    #[serde(default)]
    pub flashnet_request_id: Option<String>,
    /// Spark transfer id the receiver sees, on Spark-destination orders.
    #[serde(default)]
    pub spark_tx_hash: Option<String>,
    #[serde(default)]
    pub refund_asset: Option<String>,
    #[serde(default)]
    pub refund_amount: Option<String>,
    #[serde(default)]
    pub refund_tx_hash: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub total_fee_bps: Option<u32>,
    #[serde(default)]
    pub total_fee_amount: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[cfg(test)]
mod limits_response_tests {
    use super::*;

    /// Verbatim `/v1/orchestration/limits` entry for spark/USDB to tron/USDT,
    /// including the groups the SDK does not model.
    const LIVE_ENTRY: &str = r#"{"routes":[{
        "sourceChain":"spark","sourceAsset":"USDB",
        "destinationChain":"tron","destinationAsset":"USDT",
        "exactOutEligible":false,"fixedEligible":true,
        "source":{"chain":"spark","asset":"USDB","contractAddress":"btkn1xgrv","decimals":6,"chainId":null},
        "destination":{"chain":"tron","asset":"USDT","contractAddress":"TR7NHqje","decimals":6,"chainId":"728126428"},
        "direction":"sell",
        "limits":{
            "orderNotionalUsd":{"minCents":"80","maxCents":"9030000","source":"runtime_order_bounds"},
            "exactIn":{"supported":true,"requestAmount":{"leg":"source","chain":"spark","asset":"USDB",
                "minAmountSmallest":null,"maxAmountSmallest":null,
                "minUsdCents":"80","maxUsdCents":"9030000"},
                "constraints":["order_notional_usd","provider_dynamic_limits"]},
            "exactOut":{"supported":false,"requestAmount":null,"constraints":[]},
            "fiatUsd":null,
            "dynamicProviderLimits":{"possible":true,"components":["relay"],"description":"..."},
            "constraints":[{"id":"order_notional_usd","amountMode":"all","leg":"route"}]
        }}]}"#;

    #[test]
    fn deserializes_a_live_limits_entry() {
        let parsed: LimitsResponse = serde_json::from_str(LIVE_ENTRY).expect("live shape parses");
        let entry = &parsed.routes[0];
        // The route fields are flattened alongside `limits`.
        assert_eq!(entry.route.source_chain, "spark");
        assert_eq!(entry.route.destination.decimals, 6);

        let exact_in = entry.limits.exact_in.as_ref().expect("exactIn present");
        assert!(exact_in.supported);
        let request = exact_in.request_amount.as_ref().expect("bounds present");
        assert_eq!(request.min_amount_smallest, None);
        assert_eq!(request.min_usd_cents.as_deref(), Some("80"));
        assert_eq!(
            entry
                .limits
                .order_notional_usd
                .as_ref()
                .and_then(|b| b.max_cents.as_deref()),
            Some("9030000")
        );
    }

    #[test]
    fn survives_a_cache_round_trip() {
        // Cached responses are re-read through serde, and the flattened `route`
        // fields go through serde's buffered representation on the way back in.
        let parsed: LimitsResponse = serde_json::from_str(LIVE_ENTRY).unwrap();
        let round_tripped: LimitsResponse =
            serde_json::from_str(&serde_json::to_string(&parsed).unwrap())
                .expect("a cached response must re-read");
        let entry = &round_tripped.routes[0];
        assert_eq!(entry.route.destination.decimals, 6);
        assert!(!entry.route.exact_out_eligible);
        assert_eq!(
            entry
                .limits
                .exact_in
                .as_ref()
                .and_then(|m| m.request_amount.as_ref())
                .and_then(|r| r.min_usd_cents.as_deref()),
            Some("80")
        );
    }
}

#[cfg(test)]
mod submit_response_tests {
    use super::*;

    #[test]
    fn debug_redacts_the_read_token() {
        let response = SubmitResponse {
            order_id: "ord_1".to_string(),
            status: OrderStatus::Completed,
            read_token: Some("eyJ2IjoxLCJwIjoicGFyXzAxOWQ4NmIy".to_string()),
        };
        let rendered = format!("{response:?}");
        assert!(!rendered.contains("eyJ2"), "token leaked: {rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(rendered.contains("ord_1"), "{rendered}");
    }
}

#[cfg(test)]
mod quote_response_tests {
    use super::*;

    /// `/v1/orchestration/quote` for $50 spark/USDB to bsc/USDC, trimmed of
    /// display fields. `roundingFeeAmount` arrives in the destination's 18
    /// decimals rather than `feeAsset` units.
    const BSC_QUOTE: &str = r#"{
        "amountIn":"50000000","appFeeAmount":"49950","appFeePlatformCutAmount":"9990",
        "appFees":[{"affiliateId":"breez_sdk","amount":"49950","feeBps":10,
            "platformCutAmount":"9990","recipient":"spark1pgss","recipientAmount":"39960"}],
        "depositAddress":"spark1pgssy4sp","estimatedOut":"49850000000000000000",
        "expiresAt":"2026-09-23T07:08:28.560Z","feeAmount":"50000","feeAmountUsd":"0.05",
        "feeAsset":"USDC",
        "feeAssetDetails":{"asset":"USDC","chain":"solana",
            "chainId":"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
            "contractAddress":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","decimals":6},
        "feeBps":10,"quoteId":"q_01a0cd16","roundingFeeAmount":"5119000000000000",
        "route":["USDB","USDC"],"totalFeeAmount":"5119000000099950",
        "totalFeeAmountUsd":"5119000000.09995"
    }"#;

    #[test]
    fn deserializes_fee_asset_details_and_rounding() {
        let quote: QuoteResponse = serde_json::from_str(BSC_QUOTE).expect("live shape parses");
        let details = quote.fee_asset_details.expect("feeAssetDetails present");
        assert_eq!(details.chain.as_deref(), Some("solana"));
        assert_eq!(details.decimals, 6);
        assert_eq!(
            quote.rounding_fee_amount.as_deref(),
            Some("5119000000000000")
        );
    }

    #[test]
    fn fee_asset_details_and_rounding_are_optional() {
        let mut json: serde_json::Value = serde_json::from_str(BSC_QUOTE).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("feeAssetDetails");
        obj.remove("roundingFeeAmount");
        let quote: QuoteResponse = serde_json::from_value(json).expect("parses without them");
        assert!(quote.fee_asset_details.is_none());
        assert!(quote.rounding_fee_amount.is_none());
    }
}
