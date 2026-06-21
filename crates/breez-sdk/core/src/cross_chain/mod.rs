//! Cross-chain payment providers.
//!
//! The [`CrossChainService`] trait abstracts route discovery, quoting, and
//! sending. Each provider module (e.g. `orchestra`, `boltz`) implements it.

pub(crate) mod boltz;
pub(crate) mod boltz_event_listener;
pub(crate) mod boltz_storage_adapter;
mod cached_fiat;
mod orchestra;

pub(crate) use boltz::BoltzService;
pub(crate) use cached_fiat::{CachedFiatService, DEFAULT_FIAT_CACHE_TTL};
pub(crate) use orchestra::{BreezServerOrchestraConfigResolver, OrchestraService};

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use breez_sdk_common::fiat::FiatService;
use serde::{Deserialize, Serialize};
use spark_wallet::TransferId;

use crate::{CrossChainAddressDetails, error::SdkError};

/// SDK-level bounds for cross-chain slippage.
pub(crate) const MIN_CROSS_CHAIN_SLIPPAGE_BPS: u32 = 10;
pub(crate) const MAX_CROSS_CHAIN_SLIPPAGE_BPS: u32 = 500;
/// Used when neither the request nor [`crate::Config::default_slippage_bps`]
/// supplies a value.
pub(crate) const DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS: u32 = 100;

/// Bounds for the target-overpay pad applied to the user's destination amount
/// on `FeesExcluded` conversion sends. `0` opts out; `500` caps at 5% (matches
/// the slippage upper bound).
pub(crate) const MIN_TARGET_OVERPAY_BPS: u32 = 0;
pub(crate) const MAX_TARGET_OVERPAY_BPS: u32 = 500;
/// Default pad applied when neither the request nor
/// [`crate::CrossChainConfig::default_target_overpay_bps`] specifies one.
/// Calibrated to the observed Orchestra delivery drift; tune per provider as
/// real-world data accrues.
pub(crate) const DEFAULT_TARGET_OVERPAY_BPS: u32 = 15;
/// Tickers treated as $1-pegged for par-value rescaling. Adding a non-USD
/// ticker would silently misreport `fee_amount` for routes using it.
const USD_STABLE_ASSETS: &[&str] = &["USDB", "USDC", "USDT", "USDT0"];

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

impl From<crate::FeePolicy> for CrossChainFeeMode {
    fn from(policy: crate::FeePolicy) -> Self {
        match policy {
            crate::FeePolicy::FeesExcluded => Self::FeesExcluded,
            crate::FeePolicy::FeesIncluded => Self::FeesIncluded,
        }
    }
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

/// Per-provider service registry plus shared cross-chain dependencies (today:
/// the cached `FiatService`). Keeping the cache here scopes it to cross-chain
/// flows; `sdk.fiat_service` stays uncached for general fiat consumers.
#[derive(Clone)]
pub(crate) struct CrossChainContext {
    providers: HashMap<CrossChainProvider, Arc<dyn CrossChainService>>,
    fiat_service: Arc<dyn FiatService>,
}

impl CrossChainContext {
    pub fn new(fiat_service: Arc<dyn FiatService>) -> Self {
        Self {
            providers: HashMap::new(),
            fiat_service,
        }
    }

    pub fn insert(&mut self, key: CrossChainProvider, service: Arc<dyn CrossChainService>) {
        self.providers.insert(key, service);
    }

    /// Look up a provider, returning a friendly error if missing.
    pub fn get(
        &self,
        provider: CrossChainProvider,
    ) -> Result<&Arc<dyn CrossChainService>, SdkError> {
        self.providers.get(&provider).ok_or_else(|| {
            SdkError::InvalidInput(format!("Cross-chain provider {provider} is not available."))
        })
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<dyn CrossChainService>> {
        self.providers.values()
    }

    /// Cached fiat service shared with every cross-chain provider. Read
    /// through this on the prepare path so the TTL window is shared.
    pub fn fiat_service(&self) -> &Arc<dyn FiatService> {
        &self.fiat_service
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
        /// Spark-side deposit amount in the route's source-asset base units.
        #[serde(default)]
        deposit_amount: u128,
    },
    Boltz {
        /// Boltz swap id.
        swap_id: String,
        /// Hold invoice to pay.
        invoice: String,
        /// Hold invoice amount in sats.
        #[serde(default)]
        invoice_amount_sats: u64,
        /// Slippage tolerance in basis points.
        max_slippage_bps: u32,
    },
}

/// Data stashed on the prepared send payment so the provider can resume
/// the send stage without re-quoting.
#[derive(Debug, Clone)]
pub(crate) struct CrossChainPrepared {
    pub amount_in: u128,
    /// `amount_in` expressed in the cross-chain (destination) asset's base
    /// units, via the fiat rate or decimal rescale the SDK used at prepare
    /// time.
    pub asset_amount_in: u128,
    /// Amount the recipient will receive, in cross-chain asset base units.
    pub estimated_out: u128,
    /// Total user-visible fee in cross-chain asset base units. Covers provider
    /// spread, bridge/gas, and DEX slippage. On the token-conversion path it
    /// also rolls in the LN routing budget; on the direct path that budget
    /// lives separately in `source_transfer_fee_sats`. The dispatcher
    /// overrides this on the conversion path to reflect the token-side debit.
    pub fee_amount: u128,
    /// Provider's own service fee/spread, in its native denomination.
    pub service_fee_amount: u128,
    /// Asset that the service fee is denominated in. Unset means BTC sats.
    pub service_fee_asset: Option<String>,
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

/// Fetches the BTC/USD rate from the Breez Server fiat feed. Errors if the
/// feed is unreachable, missing the USD entry, or returns a non-finite value.
pub(crate) async fn fetch_btc_usd_rate(fiat: &dyn FiatService) -> Result<f64, SdkError> {
    let rates = fiat
        .fetch_fiat_rates()
        .await
        .map_err(|e| SdkError::Generic(format!("Cross-chain: failed to fetch fiat rates: {e}")))?;
    let btc_usd = rates
        .iter()
        .find(|r| r.coin.eq_ignore_ascii_case("USD"))
        .map(|r| r.value)
        .ok_or_else(|| {
            SdkError::Generic("Cross-chain: BTC/USD rate not found in feed".to_string())
        })?;
    if !btc_usd.is_finite() || btc_usd <= 0.0 {
        return Err(SdkError::Generic(format!(
            "Cross-chain: invalid BTC/USD rate from feed: {btc_usd}"
        )));
    }
    Ok(btc_usd)
}

/// `sats * fiat_rate * 10^dest_decimals / 10^8`. Sub-base-unit truncation
/// is absorbed by the route's slippage tolerance.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) fn convert_sats_to_destination_amount(
    sats: u128,
    fiat_rate: f64,
    dest_decimals: u32,
) -> Result<u128, SdkError> {
    let dest_scale = 10f64.powi(i32::try_from(dest_decimals).unwrap_or(i32::MAX));
    let target = (sats as f64) * fiat_rate * dest_scale / 100_000_000f64;
    if !target.is_finite() || target < 0.0 {
        return Err(SdkError::Generic(format!(
            "Cross-chain: invalid sats→dest conversion result: {target}"
        )));
    }
    Ok(target as u128)
}

pub(crate) fn is_usd_stable_asset(asset: &str) -> bool {
    USD_STABLE_ASSETS
        .iter()
        .any(|a| asset.eq_ignore_ascii_case(a))
}

/// Best-available fee: realized `asset_amount_in − delivered_amount` on
/// `Completed`, else the prepare-time estimate. Refunded/failed keep the
/// estimate (the realized formula would be misleading).
pub(crate) fn compute_terminal_fee_amount(
    new_status: &crate::ConversionStatus,
    asset_amount_in: Option<u128>,
    delivered_amount: Option<u128>,
    prepare_estimate: Option<u128>,
) -> Option<u128> {
    match (new_status, asset_amount_in, delivered_amount) {
        (crate::ConversionStatus::Completed, Some(a), Some(d)) => Some(a.saturating_sub(d)),
        _ => prepare_estimate,
    }
}

/// Rescales an amount between two base-unit precisions. Assumes
/// `1 source unit ≈ 1 dest unit` at face value — only valid for USD-stable
/// pairs. Errors on overflow.
pub(crate) fn rescale_decimals(
    amount: u128,
    src_decimals: u32,
    dest_decimals: u32,
) -> Result<u128, SdkError> {
    if dest_decimals >= src_decimals {
        let delta = dest_decimals.saturating_sub(src_decimals);
        let factor = 10u128
            .checked_pow(delta)
            .ok_or_else(|| SdkError::Generic("Cross-chain: decimal scale overflow".to_string()))?;
        amount
            .checked_mul(factor)
            .ok_or_else(|| SdkError::Generic("Cross-chain: decimal rescale overflow".to_string()))
    } else {
        let delta = src_decimals.saturating_sub(dest_decimals);
        let factor = 10u128
            .checked_pow(delta)
            .ok_or_else(|| SdkError::Generic("Cross-chain: decimal scale overflow".to_string()))?;
        amount.checked_div(factor).ok_or_else(|| {
            SdkError::Generic("Cross-chain: decimal rescale divisor zero".to_string())
        })
    }
}

/// Inverse of [`convert_sats_to_destination_amount`]: returns the sats whose
/// fiat-equivalent matches the given USD-stable `destination_amount`.
/// Errors on a non-positive `fiat_rate`.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) fn convert_destination_amount_to_sats(
    destination_amount: u128,
    fiat_rate: f64,
    dest_decimals: u32,
) -> Result<u128, SdkError> {
    if !fiat_rate.is_finite() || fiat_rate <= 0.0 {
        return Err(SdkError::Generic(format!(
            "Cross-chain: invalid BTC/USD rate for inversion: {fiat_rate}"
        )));
    }
    let dest_scale = 10f64.powi(i32::try_from(dest_decimals).unwrap_or(i32::MAX));
    let sats = (destination_amount as f64) * 100_000_000f64 / (fiat_rate * dest_scale);
    if !sats.is_finite() || sats < 0.0 {
        return Err(SdkError::Generic(format!(
            "Cross-chain: invalid dest→sats conversion result: {sats}"
        )));
    }
    Ok(sats as u128)
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

    #[test_all]
    fn convert_sats_to_destination_amount_round_trip_inverts_to_sats() {
        // 10_000 sats at $60_000/BTC → $6.00 = 6_000_000 USDC base units.
        let dest = convert_sats_to_destination_amount(10_000, 60_000.0, 6).unwrap();
        assert_eq!(dest, 6_000_000);
        // Inverse must recover the source sats.
        let sats = convert_destination_amount_to_sats(dest, 60_000.0, 6).unwrap();
        assert_eq!(sats, 10_000);
    }

    #[test_all]
    fn convert_destination_amount_to_sats_typical_stable() {
        // 1 USDC ($1.00 = 1_000_000 base units) at $60_000/BTC → 1666 sats (floor).
        let sats = convert_destination_amount_to_sats(1_000_000, 60_000.0, 6).unwrap();
        assert_eq!(sats, 1_666);
    }

    #[test_all]
    fn convert_destination_amount_to_sats_zero_passes_through() {
        let sats = convert_destination_amount_to_sats(0, 60_000.0, 6).unwrap();
        assert_eq!(sats, 0);
    }

    #[test_all]
    fn convert_destination_amount_to_sats_rejects_non_positive_rate() {
        let err = convert_destination_amount_to_sats(1_000_000, 0.0, 6).unwrap_err();
        assert!(matches!(err, SdkError::Generic(ref m) if m.contains("invalid BTC/USD rate")));
        let err = convert_destination_amount_to_sats(1_000_000, f64::NAN, 6).unwrap_err();
        assert!(matches!(err, SdkError::Generic(_)));
    }

    #[test_all]
    fn rescale_decimals_scales_down_when_dest_decimals_lower() {
        assert_eq!(rescale_decimals(100_000_000, 8, 6).unwrap(), 1_000_000);
    }

    #[test_all]
    fn rescale_decimals_same_decimals_is_identity() {
        assert_eq!(rescale_decimals(123_456_789, 6, 6).unwrap(), 123_456_789);
    }

    #[test_all]
    fn rescale_decimals_scales_up_when_dest_decimals_higher() {
        assert_eq!(rescale_decimals(1_000_000, 6, 8).unwrap(), 100_000_000);
    }

    #[test_all]
    fn rescale_decimals_zero_passes_through() {
        assert_eq!(rescale_decimals(0, 8, 6).unwrap(), 0);
        assert_eq!(rescale_decimals(0, 6, 8).unwrap(), 0);
    }

    #[test_all]
    fn is_usd_stable_asset_recognizes_known_stables() {
        for ticker in ["USDB", "USDC", "USDT", "USDT0", "usdb", "uSdC"] {
            assert!(is_usd_stable_asset(ticker), "{ticker} should be stable");
        }
    }

    #[test_all]
    fn is_usd_stable_asset_rejects_btc_and_unknown() {
        for ticker in ["BTC", "ETH", "DAI", "", "USD"] {
            assert!(
                !is_usd_stable_asset(ticker),
                "{ticker} should not be a recognized USD-stable"
            );
        }
    }

    // ---- compute_terminal_fee_amount ----

    #[test_all]
    fn compute_terminal_fee_overwrites_estimate_on_completed() {
        let realized = compute_terminal_fee_amount(
            &crate::ConversionStatus::Completed,
            Some(1_020_434), // asset_amount_in
            Some(997_498),   // delivered_amount
            Some(20_434),    // prepare-time estimate
        );
        assert_eq!(realized, Some(22_936), "= asset_amount_in − delivered");
    }

    #[test_all]
    fn compute_terminal_fee_keeps_estimate_on_refunded() {
        // Refunded payments don't have a realized fee semantic; the estimate
        // is the best we can show (and the realized formula would produce
        // garbage because delivered_amount is 0/None on a refund).
        let realized = compute_terminal_fee_amount(
            &crate::ConversionStatus::Refunded,
            Some(1_020_434),
            None,
            Some(20_434),
        );
        assert_eq!(realized, Some(20_434));
    }

    #[test_all]
    fn compute_terminal_fee_keeps_estimate_on_failed() {
        let realized = compute_terminal_fee_amount(
            &crate::ConversionStatus::Failed,
            Some(1_020_434),
            None,
            Some(20_434),
        );
        assert_eq!(realized, Some(20_434));
    }

    #[test_all]
    fn compute_terminal_fee_keeps_estimate_when_asset_amount_in_missing() {
        // Pre-upgrade rows have no asset_amount_in; realized fee can't be
        // computed, so the stored estimate stays as-is.
        let realized = compute_terminal_fee_amount(
            &crate::ConversionStatus::Completed,
            None, // asset_amount_in missing
            Some(997_498),
            Some(20_434),
        );
        assert_eq!(realized, Some(20_434));
    }

    #[test_all]
    fn compute_terminal_fee_keeps_estimate_when_delivered_amount_missing() {
        // Should never happen on Completed per the contract, but defend
        // against the edge anyway.
        let realized = compute_terminal_fee_amount(
            &crate::ConversionStatus::Completed,
            Some(1_020_434),
            None, // delivered_amount missing
            Some(20_434),
        );
        assert_eq!(realized, Some(20_434));
    }

    #[test_all]
    fn compute_terminal_fee_saturating_sub_on_over_delivery() {
        // Rare but possible: provider over-delivers vs source.
        let realized = compute_terminal_fee_amount(
            &crate::ConversionStatus::Completed,
            Some(1_000_000),
            Some(1_005_000),
            Some(0),
        );
        assert_eq!(
            realized,
            Some(0),
            "saturating_sub must clamp at 0, not underflow"
        );
    }

    /// Regression: `CrossChainProviderContext::Boltz.invoice_amount_sats` must
    /// be the source of truth for the LN-leg amount, distinct from
    /// `CrossChainPrepared::amount_in` (which can carry a user-facing display
    /// value such as token base units after the dispatcher's conversion-path
    /// override). Conflating the two persisted USDB base units into the
    /// `invoice_amount_sats` field of `ConversionInfo::Boltz`, showing a
    /// ~$1,200,000-sat "from" amount for a ~$1 send. This test asserts the
    /// two fields are independently representable + survive a serde
    /// round-trip with their distinct values.
    #[test_all]
    fn boltz_provider_context_invoice_amount_sats_is_independent_of_amount_in() {
        let ctx = CrossChainProviderContext::Boltz {
            swap_id: "swap_1".to_string(),
            invoice: "lnbc19090n1pexample".to_string(),
            invoice_amount_sats: 1_909,
            max_slippage_bps: 100,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let decoded: CrossChainProviderContext = serde_json::from_str(&json).unwrap();
        let CrossChainProviderContext::Boltz {
            invoice_amount_sats,
            ..
        } = &decoded
        else {
            panic!("expected Boltz variant");
        };
        assert_eq!(*invoice_amount_sats, 1_909);
        assert!(
            *invoice_amount_sats != 1_222_703,
            "the LN invoice sats must never be conflated with a user-facing display value (e.g. USDB base units)"
        );
    }

    /// Pre-bug-fix persisted contexts lack `invoice_amount_sats`. Serde must
    /// default the missing field to 0 rather than failing to deserialize — the
    /// downstream send-time error becomes obvious instead of corrupting the
    /// stored Payment.
    #[test_all]
    fn boltz_provider_context_legacy_row_without_invoice_amount_sats_defaults_to_zero() {
        let legacy = r#"{
            "Boltz": {
                "swap_id": "swap_legacy",
                "invoice": "lnbc19090n1p",
                "max_slippage_bps": 100
            }
        }"#;
        let decoded: CrossChainProviderContext = serde_json::from_str(legacy).unwrap();
        let CrossChainProviderContext::Boltz {
            invoice_amount_sats,
            ..
        } = &decoded
        else {
            panic!("expected Boltz variant");
        };
        assert_eq!(*invoice_amount_sats, 0);
    }

    /// Same invariant for Orchestra: `deposit_amount` is the source of truth
    /// for the deposit transfer size and is distinct from
    /// `CrossChainPrepared::amount_in`.
    #[test_all]
    fn orchestra_provider_context_deposit_amount_is_independent_of_amount_in() {
        let ctx = CrossChainProviderContext::Orchestra {
            quote_id: "q_1".to_string(),
            deposit_address: "spark1...".to_string(),
            deposit_amount: 1_020_434,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let decoded: CrossChainProviderContext = serde_json::from_str(&json).unwrap();
        let CrossChainProviderContext::Orchestra { deposit_amount, .. } = &decoded else {
            panic!("expected Orchestra variant");
        };
        assert_eq!(*deposit_amount, 1_020_434);
    }
}
