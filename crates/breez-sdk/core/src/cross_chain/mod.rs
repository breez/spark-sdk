//! Cross-chain payment providers.
//!
//! The [`CrossChainService`] trait abstracts route discovery, quoting, and
//! sending. Each provider module (e.g. `orchestra`, `boltz`) implements it.

pub(crate) mod boltz;
pub(crate) mod boltz_event_listener;
pub(crate) mod boltz_storage_adapter;
mod orchestra;

pub(crate) use boltz::BoltzService;
pub(crate) use orchestra::{BreezServerOrchestraConfigResolver, OrchestraService};

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use spark_wallet::TransferId;

use crate::{CrossChainAddressDetails, error::SdkError};

/// Resolves the BTC-leg [`TransferId`] for a cross-chain send. A
/// caller-supplied `idempotency_key` from [`crate::SendPaymentRequest`]
/// wins so the top-level `get_payment_by_id(idempotency_key)` lookup in
/// `orchestrate_send` can short-circuit retries; otherwise we derive a
/// `UUIDv5` from `fallback_seed` (the provider's quote/swap id) so that
/// re-sending the same prepared shape still hits Spark's protocol-level
/// dedup. Mirrors the stable-balance per-receive convention. Token-source
/// sends ignore the return value: [`spark_wallet::transfer_tokens`] has
/// no idempotency hook.
pub(crate) fn derive_btc_leg_transfer_id(
    idempotency_key: Option<&str>,
    fallback_seed: &str,
) -> Result<TransferId, SdkError> {
    match idempotency_key {
        Some(key) => TransferId::from_str(key).map_err(SdkError::Generic),
        None => Ok(TransferId::from_name(fallback_seed)),
    }
}

/// SDK-level bounds for cross-chain slippage.
pub(crate) const MIN_CROSS_CHAIN_SLIPPAGE_BPS: u32 = 10;
pub(crate) const MAX_CROSS_CHAIN_SLIPPAGE_BPS: u32 = 500;
/// Used when neither the request nor [`crate::Config::default_slippage_bps`]
/// supplies a value.
pub(crate) const DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CrossChainProvider {
    Orchestra,
    Boltz,
}

impl std::fmt::Display for CrossChainProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Orchestra => f.write_str("Orchestra"),
            Self::Boltz => f.write_str("Boltz"),
        }
    }
}

/// The source asset a cross-chain route accepts as input on the Spark side.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SourceAsset {
    /// Native BTC (sats).
    Bitcoin,
    /// A Spark token, identified by its bech32m `token_identifier` (e.g. `btkn1...`).
    Token { token_identifier: String },
}

/// How the caller wants fees handled against the request `amount`.
///
/// - `FeesExcluded`: `amount` is the provider invoice/deposit target; the
///   wallet pays `amount + source_transfer_fee_sats` in total.
/// - `FeesIncluded`: `amount` is the wallet's total sats budget; the provider
///   leg is sized so `amount_in + source_transfer_fee_sats <= amount`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CrossChainFeeMode {
    FeesExcluded,
    FeesIncluded,
}

/// Filter for [`CrossChainService::get_routes`] and the public
/// `get_cross_chain_routes()` API.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CrossChainRouteFilter {
    /// Routes for sending from Spark to another chain.
    /// Filtered by the parsed recipient address details.
    Send {
        address_details: CrossChainAddressDetails,
    },
    /// Routes for receiving to Spark from another chain.
    /// Optionally filtered by the source token contract address.
    Receive { contract_address: Option<String> },
}

/// A single route available for cross-chain transfers, tagged with the provider
/// that offers it. Returned by `get_cross_chain_routes()`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CrossChainRoutePair {
    /// Which provider offers this route.
    pub provider: CrossChainProvider,
    /// Destination blockchain (e.g. `"base"`, `"solana"`, `"tron"`).
    pub chain: String,
    /// Stable chain identifier (e.g. EVM `chainId` as a decimal string).
    /// `None` for non-EVM chains that don't expose one, or when the
    /// provider doesn't surface it.
    pub chain_id: Option<String>,
    /// Destination asset symbol (e.g. `"USDC"`, `"USDT"`).
    pub asset: String,
    /// Token contract / mint address on the destination chain.
    pub contract_address: Option<String>,
    /// Decimal places for the destination asset.
    pub decimals: u8,
    /// Whether the route supports exact-out mode.
    pub exact_out_eligible: bool,
    /// The source assets this route accepts on the Spark side.
    ///
    /// Boltz routes accept `[SourceAsset::Bitcoin]`. Orchestra routes accept
    /// one or more of `Bitcoin` / `Token(...)` (a given destination endpoint
    /// may be fronted by multiple source variants on Orchestra).
    pub supported_sources: Vec<SourceAsset>,
}

impl CrossChainRoutePair {
    /// Infers the destination address family from the route's
    /// `contract_address`. Returns `None` for native-asset routes (no
    /// contract address) or if the address format isn't recognized; callers
    /// should treat that as "skip the address-family validation".
    pub(crate) fn destination_address_family(
        &self,
    ) -> Option<breez_sdk_common::input::CrossChainAddressFamily> {
        self.contract_address
            .as_deref()
            .and_then(breez_sdk_common::input::detect_address_family)
    }
}

/// Registry of cross-chain providers keyed by [`CrossChainProvider`].
#[derive(Clone, Default)]
pub(crate) struct CrossChainProviders(HashMap<CrossChainProvider, Arc<dyn CrossChainService>>);

impl CrossChainProviders {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn insert(&mut self, key: CrossChainProvider, service: Arc<dyn CrossChainService>) {
        self.0.insert(key, service);
    }

    /// Look up a provider, returning a friendly error if missing.
    pub fn get(
        &self,
        provider: CrossChainProvider,
    ) -> Result<&Arc<dyn CrossChainService>, SdkError> {
        self.0.get(&provider).ok_or_else(|| {
            SdkError::InvalidInput(format!("Cross-chain provider {provider} is not available."))
        })
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<dyn CrossChainService>> {
        self.0.values()
    }
}

/// Provider-internal state produced by `prepare` and consumed by `send`.
/// Typed per provider so the send stage can resume without re-quoting and
/// without a serde round-trip. Callers should round-trip this value as-is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CrossChainProviderContext {
    Orchestra {
        /// Orchestra quote id, passed back on `/submit`.
        quote_id: String,
        /// Spark address Orchestra expects the deposit transfer to land on.
        deposit_address: String,
    },
    Boltz {
        /// Boltz swap id.
        swap_id: String,
        /// Hold invoice to pay. The invoice amount matches
        /// [`CrossChainPrepared::amount_in`].
        invoice: String,
        /// Slippage tolerance applied to this swap, in basis points.
        max_slippage_bps: u32,
    },
}

/// Data stashed on the prepared send payment so the provider can resume
/// the send stage without re-quoting.
#[derive(Debug, Clone)]
pub(crate) struct CrossChainPrepared {
    pub amount_in: u128,
    /// Amount the recipient will receive, already net of any destination-chain
    /// costs (gas, bridge messaging). Destination-side costs are implicit in
    /// `amount_in - estimated_out` and are not re-counted in `fee_amount`.
    pub estimated_out: u128,
    /// Sender-side service fee charged by the provider. Excludes
    /// destination-chain costs, which are already deducted from `estimated_out`.
    pub fee_amount: u128,
    /// The asset the fee is denominated in. `None` means BTC (sats).
    pub fee_asset: Option<String>,
    /// Sats cost to the wallet of moving `amount_in` from the wallet to the
    /// provider. For Boltz: the Lightning routing fee budget for paying the
    /// hold invoice (a budget, not a central estimate — enforced as a hard
    /// cap at send time). For Orchestra: the Spark transfer fee (0 today;
    /// non-zero in the future).
    ///
    /// Semantically distinct from `fee_amount` (provider's service fee /
    /// spread) and from destination-chain costs (baked into `estimated_out`).
    /// Denominated in sats — the field assumes a sats-denominated source leg.
    pub source_transfer_fee_sats: u64,
    /// Fee mode the prepare was called with. Needed at send time so the
    /// provider knows whether to apply FeesIncluded-style overpayment.
    pub fee_mode: CrossChainFeeMode,
    pub expires_at: String,
    pub pair: CrossChainRoutePair,
    pub recipient_address: String,
    /// The `token_identifier` on the Spark source (e.g. USDB). `None` for BTC sats.
    pub token_identifier: Option<String>,
    /// Provider-internal state carried between `prepare` and `send`.
    pub provider_context: CrossChainProviderContext,
}

/// Abstraction over cross-chain bridge/swap providers.
///
/// Each implementation owns its own client, caching, and background monitoring.
/// The SDK dispatches to the provider via this trait.
#[macros::async_trait]
pub(crate) trait CrossChainService: Send + Sync {
    /// Returns the available cross-chain route pairs.
    ///
    /// The returned [`CrossChainRoutePair`] always describes the non-Spark
    /// side of the route. The [`CrossChainRouteFilter`] controls direction
    /// and optional filtering.
    async fn get_routes(
        &self,
        filter: &CrossChainRouteFilter,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError>;

    /// Fetch a quote for a cross-chain send.
    async fn prepare(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        source_token_identifier: Option<String>,
        max_slippage_bps: u32,
        fee_mode: CrossChainFeeMode,
    ) -> Result<CrossChainPrepared, SdkError>;

    /// Execute the send: transfer funds to the deposit address, submit to
    /// the provider, persist metadata, monitor to terminal, and return the
    /// resulting [`Payment`].
    ///
    /// `idempotency_key` is the caller-provided key from `SendPaymentRequest`.
    /// Providers should use it as the underlying Spark `TransferId` so the
    /// outbound transfer is protocol-level idempotent on retry; if `None`,
    /// the provider derives a deterministic key from its own quote/swap id
    /// (same shape as the stable-balance per-receive convention). Only the
    /// BTC-source branch benefits — token transfers have no upstream
    /// idempotency hook, and the top-level dispatcher already rejects
    /// idempotency keys for token-source sends.
    ///
    /// Each provider owns the polling-to-terminal step internally — the
    /// SDK dispatcher does not wrap this with an additional wait.
    async fn send(
        &self,
        prepared: &CrossChainPrepared,
        idempotency_key: Option<String>,
    ) -> Result<crate::Payment, SdkError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn derive_btc_leg_transfer_id_uses_caller_key() {
        // A v4 UUID is a valid TransferId — using one here checks that the
        // caller-supplied key wins outright.
        let key = "00000000-0000-4000-8000-000000000001";
        let id = derive_btc_leg_transfer_id(Some(key), "ignored-seed").unwrap();
        assert_eq!(id.to_string(), key);
    }

    #[test_all]
    fn derive_btc_leg_transfer_id_deterministic_from_seed() {
        let a = derive_btc_leg_transfer_id(None, "cross_chain:orchestra:quote-1").unwrap();
        let b = derive_btc_leg_transfer_id(None, "cross_chain:orchestra:quote-1").unwrap();
        assert_eq!(
            a, b,
            "same seed must produce the same TransferId across calls"
        );
    }

    #[test_all]
    fn derive_btc_leg_transfer_id_distinct_seeds_yield_distinct_ids() {
        let a = derive_btc_leg_transfer_id(None, "cross_chain:orchestra:quote-1").unwrap();
        let b = derive_btc_leg_transfer_id(None, "cross_chain:orchestra:quote-2").unwrap();
        assert_ne!(a, b);
    }

    #[test_all]
    fn derive_btc_leg_transfer_id_orchestra_and_boltz_seeds_collide_only_on_id() {
        // The provider tag in the seed prevents an Orchestra `quote-1` and a
        // hypothetical Boltz `quote-1` from colliding on the same TransferId.
        let orchestra = derive_btc_leg_transfer_id(None, "cross_chain:orchestra:abc").unwrap();
        let boltz = derive_btc_leg_transfer_id(None, "cross_chain:boltz:abc").unwrap();
        assert_ne!(orchestra, boltz);
    }

    #[test_all]
    fn derive_btc_leg_transfer_id_rejects_invalid_caller_key() {
        let err = derive_btc_leg_transfer_id(Some("not-a-uuid"), "fallback").unwrap_err();
        assert!(matches!(err, SdkError::Generic(_)));
    }
}
