//! Request/response types for the Flashnet Orchestra API.
//!
//! Mirrors the public schema at
//! <https://docs.flashnet.xyz/products/orchestration/api/quotes-and-orders>.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// GET /v1/orchestration/routes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutesResponse {
    pub routes: Vec<Route>,
}

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
    pub route: Vec<String>,
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
    pub total_fee_amount: String,
    #[serde(default)]
    pub app_fee_amount: Option<String>,
    #[serde(default)]
    pub app_fee_platform_cut_amount: Option<String>,
    #[serde(default)]
    pub app_fees: Vec<AppFeeResult>,
    pub fee_asset: String,
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
pub struct SubmitRequestSpark {
    pub quote_id: String,
    pub spark_tx_hash: String,
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
    pub quote_id: String,
    pub source_chain: String,
    pub source_asset: String,
    #[serde(default)]
    pub source_address: Option<String>,
    pub source_tx_hash: String,
    #[serde(default)]
    pub source_tx_vout: Option<u32>,
    pub deposit_address: String,
    pub destination_chain: String,
    pub destination_asset: String,
    pub recipient_address: String,
    pub amount_in: String,
    #[serde(default)]
    pub amount_out: Option<String>,
    pub fee_bps: u32,
    pub fee_amount: String,
    pub slippage_bps: u32,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
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
