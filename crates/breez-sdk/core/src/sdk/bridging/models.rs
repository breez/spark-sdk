use serde::{Deserialize, Serialize};

use crate::{
    CrossChainConfig, CrossChainProvider, CrossChainReceiveInfo, CrossChainRouteLimits, FeePolicy,
    ProxyConfig,
};

/// Settings for a [`BridgingClient`](super::BridgingClient) built with
/// [`new_bridging_client`](super::new_bridging_client).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BridgingConfig {
    /// Breez API key.
    pub api_key: String,
    /// Routes the client's connections through a SOCKS5 proxy.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub proxy: Option<ProxyConfig>,
    /// Default slippage and overpay. Unset uses the built-in defaults.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub cross_chain_config: Option<CrossChainConfig>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetBridgeRoutesRequest {
    /// Who to pay: a Spark address, a Lightning address (a Cash App
    /// `$cashtag` included), or an EVM/Solana/Tron address or URI.
    pub recipient: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetBridgeRoutesResponse {
    /// The routes that can pay the recipient. Empty when none can.
    pub routes: Vec<BridgeRoutePair>,
}

/// A way to pay a recipient: what the payer pays, and what the recipient
/// receives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BridgeRoutePair {
    pub provider: CrossChainProvider,
    /// What the payer pays.
    pub source: BridgeRoutePairSide,
    /// What the recipient receives.
    pub destination: BridgeRoutePairSide,
    /// Bounds on what the payer pays, in `source` base units. Unset when the
    /// provider publishes none.
    pub limits: Option<CrossChainRouteLimits>,
}

/// One side of a [`BridgeRoutePair`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BridgeRoutePairSide {
    /// `"spark"`, `"lightning"`, or another chain, e.g. `"base"` or `"tron"`.
    pub chain: String,
    /// EVM `chainId` as a decimal string. Unset on chains without one.
    pub chain_id: Option<String>,
    /// Asset symbol, e.g. `"USDC"`, `"USDT"`, `"BTC"` or `"USDB"`.
    pub asset: String,
    /// Token contract or mint address, or the token identifier on Spark.
    /// Unset for Bitcoin.
    pub contract_address: Option<String>,
    pub decimals: u8,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreateBridgeRequest {
    /// Who to pay, as given to [`BridgingClient::get_routes`](super::BridgingClient::get_routes).
    pub recipient: String,
    /// A route from [`BridgingClient::get_routes`](super::BridgingClient::get_routes)
    /// for this recipient.
    pub route: BridgeRoutePair,
    /// Amount in stablecoin base units: the destination asset when the
    /// recipient is paid in a stablecoin, the source asset otherwise, per that
    /// side's `decimals`. At parity this is the USD value: `1_000_000` is 1
    /// USDC at 6 decimals.
    pub amount: u128,
    /// The payer's address on the source chain, where the deposit is refunded
    /// if the recipient can't be paid. Required to pay a Lightning address,
    /// and recommended whenever the payer pays from another chain: without
    /// one the provider has no address to refund to. Ignored when the payer
    /// pays over Lightning.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub refund_address: Option<String>,
    /// Whether fees are added on top of `amount` (`FeesExcluded`, the default)
    /// or deducted from it (`FeesIncluded`).
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub fee_policy: Option<FeePolicy>,
    /// Maximum slippage tolerance in basis points. Unset uses the configured
    /// default.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub max_slippage_bps: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CreateBridgeResponse {
    /// Identifies the bridge for [`BridgingClient::get_bridge_status`](super::BridgingClient::get_bridge_status).
    /// Store it to look up the status later.
    pub bridge_id: String,
    /// Pass it to
    /// [`BridgingClient::get_bridge_status`](super::BridgingClient::get_bridge_status).
    pub status_token: Option<String>,
    /// What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice or a
    /// Cash App link over Lightning, otherwise the bare deposit address.
    pub payment_request: String,
    /// The deposit and the expected delivery.
    pub info: CrossChainReceiveInfo,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetBridgeStatusRequest {
    pub bridge_id: String,
    /// The bridge's status token, if any.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub status_token: Option<String>,
}

/// Where a bridge is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum BridgeStatus {
    /// The bridge was created, and the payer hasn't paid yet. An unpaid bridge
    /// stays `Created` after its `expires_at`. A late deposit on another chain
    /// may still be accepted at a new price.
    Created,
    /// The provider is processing the payment, from confirming the deposit to
    /// paying the recipient. `provider_status` gives the stage.
    Pending,
    /// The recipient can't be paid, and the payment is being refunded.
    Refunding,
    /// The recipient was paid.
    Completed,
    /// The payment was refunded to the payer's refund address.
    Refunded,
    /// The bridge failed. `error_code` gives the reason.
    Failed,
    /// The provider's order expired without a payment.
    Expired,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetBridgeStatusResponse {
    pub status: BridgeStatus,
    /// The provider's own status, e.g. `"bridging"` or `"awaiting_approval"`,
    /// for display and support. Unset before the payment arrives.
    pub provider_status: Option<String>,
    /// The provider's order id, once the payment arrived.
    pub order_id: Option<String>,
    /// The amount the payer paid, in the source asset's base units.
    pub amount_in: Option<u128>,
    /// The amount the recipient received, in the destination asset's base
    /// units. Set once completed.
    pub amount_out: Option<u128>,
    /// The payer's transaction on the source chain.
    pub deposit_tx_hash: Option<String>,
    /// The transaction that paid the recipient, or the Spark transfer when the
    /// recipient is on Spark. Set once completed.
    pub delivery_tx_hash: Option<String>,
    /// The transaction that refunded the payer.
    pub refund_tx_hash: Option<String>,
    /// The provider's error code when the bridge failed.
    pub error_code: Option<String>,
}
