//! Sanity checks on server-supplied pool data.
//!
//! `f64` pool fields parse with `FromStr`, so they accept `NaN`, `inf` and
//! overflow literals, and the casts downstream saturate (NaN to 0, `+inf` to
//! `u128::MAX`). Nothing else range-checks them, so a crafted pool can drive
//! the quote to zero or to an arbitrary `amount_in`.

use spark::Network;

use super::models::{CurveType, Pool};
use super::utils::decode_token_identifier;
use crate::error::FlashnetError;

/// Excludes degenerate values rather than expressing a view on pricing. The
/// price is denominated in base units, so its magnitude scales with the token's
/// decimals: a cheap or high-decimal token is legitimately tiny, and a tighter
/// bound would reject it over a field the quote may never read.
const MIN_PRICE: f64 = 1e-30;
const MAX_PRICE: f64 = 1e30;
/// Beyond this it is corrupt data, not a price move. Feeds a stability score
/// where a NaN would otherwise score best.
const MAX_PRICE_CHANGE_PCT: f64 = 1e9;
/// Total pool fees cannot exceed the principal.
const MAX_TOTAL_FEE_BPS: u32 = 10_000;
/// Concentrated liquidity quotes price as `1.0001^tick`, one tick being a
/// single basis point of price.
const TICK_BASE: f64 = 1.0001;
/// How far the advertised price may sit from the tick that encodes it. The live
/// mainnet pool differs by under 1 bp, so this is roughly 100x the observed
/// error and cannot bite a pool whose two fields agree.
const TICK_TOLERANCE_BPS: u32 = 100;
/// How far the advertised price may sit from the reserve-implied one. Wider
/// than the tick bound because reserves and price need not be sampled at the
/// same instant; the live pool differs by 0.016 bps.
const RESERVE_TOLERANCE_BPS: u32 = 500;

/// Why a pool's advertised data cannot be trusted.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolValidationError {
    NonFiniteField {
        field: &'static str,
    },
    PriceOutOfRange {
        field: &'static str,
        value: f64,
    },
    FeeOutOfRange {
        host_fee_bps: u32,
        lp_fee_bps: u32,
    },
    AssetPairMismatch {
        asset_in: String,
        asset_out: String,
    },
    /// Names a curve this client is unaware of, or names none at all.
    UnsupportedCurve,
    /// Claims the preferred curve type without the tick that would corroborate
    /// it, or with one the advertised price contradicts.
    UncorroboratedCurve,
    /// The advertised price disagrees with the reserves behind it.
    PriceContradictsReserves,
}

impl std::fmt::Display for PoolValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolValidationError::NonFiniteField { field } => {
                write!(f, "{field} is not a finite number")
            }
            PoolValidationError::PriceOutOfRange { field, value } => {
                write!(f, "{field} is out of range: {value}")
            }
            PoolValidationError::FeeOutOfRange {
                host_fee_bps,
                lp_fee_bps,
            } => write!(
                f,
                "fees out of range: host {host_fee_bps} bps, lp {lp_fee_bps} bps"
            ),
            PoolValidationError::AssetPairMismatch {
                asset_in,
                asset_out,
            } => write!(f, "pool does not trade the pair {asset_in} to {asset_out}"),
            PoolValidationError::UnsupportedCurve => {
                write!(f, "curve is unrecognised or unstated")
            }
            PoolValidationError::UncorroboratedCurve => {
                write!(f, "declares concentrated liquidity without a matching tick")
            }
            PoolValidationError::PriceContradictsReserves => {
                write!(f, "advertised price disagrees with the reserves")
            }
        }
    }
}

impl From<PoolValidationError> for FlashnetError {
    fn from(err: PoolValidationError) -> Self {
        FlashnetError::Generic(format!("Invalid pool data: {err}"))
    }
}

/// Range comparison rather than equality: clippy's pedantic lints reject float
/// `==`, and NaN compares false against every bound regardless.
fn check_finite(
    value: Option<f64>,
    field: &'static str,
    range: Option<(f64, f64)>,
) -> Result<(), PoolValidationError> {
    let Some(value) = value else {
        return Ok(());
    };
    if !value.is_finite() {
        return Err(PoolValidationError::NonFiniteField { field });
    }
    if let Some((min, max)) = range
        && (value < min || value > max)
    {
        return Err(PoolValidationError::PriceOutOfRange { field, value });
    }
    Ok(())
}

impl Pool {
    /// Structural sanity of the advertised numbers, independent of any swap.
    ///
    /// Reserves are deliberately unchecked: empty pools are normal, and
    /// [`Pool::calculate_amount_in`] already refuses them.
    pub fn validate(&self) -> Result<(), PoolValidationError> {
        check_finite(
            self.current_price_a_in_b,
            "current_price_a_in_b",
            Some((MIN_PRICE, MAX_PRICE)),
        )?;
        check_finite(
            self.price_change_percent_24h,
            "price_change_percent_24h",
            Some((-MAX_PRICE_CHANGE_PCT, MAX_PRICE_CHANGE_PCT)),
        )?;
        check_finite(
            self.bonding_progress_percent,
            "bonding_progress_percent",
            None,
        )?;

        if self.host_fee_bps > MAX_TOTAL_FEE_BPS
            || self.lp_fee_bps > MAX_TOTAL_FEE_BPS
            || self.host_fee_bps.saturating_add(self.lp_fee_bps) > MAX_TOTAL_FEE_BPS
        {
            return Err(PoolValidationError::FeeOutOfRange {
                host_fee_bps: self.host_fee_bps,
                lp_fee_bps: self.lp_fee_bps,
            });
        }

        if matches!(self.curve_type, Some(CurveType::Unknown) | None) {
            return Err(PoolValidationError::UnsupportedCurve);
        }

        // A concentrated-liquidity claim discards every other curve type from
        // selection and skips the reserve maths, so it has to be corroborated
        // by the tick.
        if self.curve_type == Some(CurveType::V3Concentrated)
            && self.price_matches_tick(TICK_TOLERANCE_BPS) != Some(true)
        {
            return Err(PoolValidationError::UncorroboratedCurve);
        }
        if self.price_matches_reserves(RESERVE_TOLERANCE_BPS) == Some(false) {
            return Err(PoolValidationError::PriceContradictsReserves);
        }

        Ok(())
    }

    /// [`Pool::validate`] plus confirmation that the pool actually trades the
    /// requested pair. Without this, an unrecognised `asset_in` is silently
    /// treated as asset B and quoted in the wrong direction.
    pub fn validate_for_swap(
        &self,
        asset_in_address: &str,
        asset_out_address: &str,
        network: Network,
    ) -> Result<(), FlashnetError> {
        self.validate()?;

        let asset_in = decode_token_identifier(asset_in_address, network)?;
        let asset_out = decode_token_identifier(asset_out_address, network)?;
        let matches_pair = (asset_in == self.asset_a_address && asset_out == self.asset_b_address)
            || (asset_in == self.asset_b_address && asset_out == self.asset_a_address);
        if !matches_pair {
            return Err(PoolValidationError::AssetPairMismatch {
                asset_in,
                asset_out,
            }
            .into());
        }

        Ok(())
    }

    /// Whether the advertised price agrees with the reserves. `None` when no
    /// check is possible.
    ///
    /// Judges each curve by the reserves it actually prices against, which is
    /// what [`Pool::quote_reserves`] returns: the real pair for constant
    /// product, the traversed virtual pair for a bonding curve. A pool cannot
    /// escape the check by naming a curve, since naming one selects its
    /// identity rather than an exemption. Concentrated liquidity is excluded,
    /// because no reserve pair implies its price, and
    /// [`Pool::price_matches_tick`] covers it instead.
    #[must_use]
    pub fn price_matches_reserves(&self, tolerance_bps: u32) -> Option<bool> {
        if self.curve_type == Some(CurveType::V3Concentrated) {
            return None;
        }
        let price = self.current_price_a_in_b?;
        if !price.is_finite() || price <= 0.0 {
            return None;
        }
        let implied = self.implied_price_a_in_b()?;
        let tolerance = f64::from(tolerance_bps) / 10_000.0;
        Some((implied - price).abs() <= price * tolerance)
    }

    /// Whether the advertised price agrees with the concentrated-liquidity tick,
    /// which encodes it as `1.0001^tick`. `None` when either field is absent.
    ///
    /// The counterpart to [`Pool::price_matches_reserves`] for the curve that
    /// check cannot judge. Catches inconsistent data, not a coherent
    /// counterparty: one supplier provides both fields.
    #[must_use]
    pub fn price_matches_tick(&self, tolerance_bps: u32) -> Option<bool> {
        let price = self.current_price_a_in_b?;
        let tick = self.current_tick?;
        if !price.is_finite() || price <= 0.0 {
            return None;
        }

        let from_tick = TICK_BASE.powi(tick);
        if !from_tick.is_finite() || from_tick <= 0.0 {
            return None;
        }
        // Pools do not order their pair consistently, so the tick may encode
        // either side of it. The live listing carries both orientations.
        let tolerance = f64::from(tolerance_bps) / 10_000.0;
        let matches = |p: f64| (from_tick - p).abs() <= p * tolerance;
        Some(matches(price) || matches(1.0 / price))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amm::api::BTC_ASSET_ADDRESS;
    use std::str::FromStr;

    const USDB: &str = "3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca";

    fn pool() -> Pool {
        Pool {
            lp_public_key: spark_wallet::PublicKey::from_str(
                "02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
            )
            .unwrap(),
            host_name: "flashnet".to_string(),
            host_fee_bps: 0,
            lp_fee_bps: 20,
            asset_a_address: BTC_ASSET_ADDRESS.to_string(),
            asset_b_address: USDB.to_string(),
            asset_a_reserve: Some(260_745),
            asset_b_reserve: Some(171_239_613),
            virtual_reserve_a: None,
            virtual_reserve_b: None,
            threshold_pct: None,
            current_price_a_in_b: Some(656.733_136_003_420_6),
            tvl_asset_b: Some(342_479_495),
            volume_24h_asset_b: Some(4_041_946),
            price_change_percent_24h: Some(-2.53),
            curve_type: Some(CurveType::ConstantProduct),
            initial_reserve_a: None,
            bonding_progress_percent: None,
            graduation_threshold_amount: None,
            created_at: "2025-09-22 19:09:36".to_string(),
            updated_at: "2026-08-04 15:03:38".to_string(),
            current_tick: None,
            tick_spacing: None,
            total_liquidity: None,
        }
    }

    #[test]
    fn a_live_pool_validates() {
        assert!(pool().validate().is_ok());
    }

    #[test]
    fn empty_reserves_are_not_a_validation_failure() {
        // Most live pools report zero on both sides.
        let mut p = pool();
        p.asset_a_reserve = Some(0);
        p.asset_b_reserve = Some(0);
        assert!(p.validate().is_ok());

        p.asset_a_reserve = None;
        p.asset_b_reserve = None;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn non_finite_price_is_rejected() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut p = pool();
            p.current_price_a_in_b = Some(value);
            assert_eq!(
                p.validate(),
                Err(PoolValidationError::NonFiniteField {
                    field: "current_price_a_in_b"
                })
            );
        }
    }

    #[test]
    fn zero_or_negative_price_is_rejected() {
        for value in [0.0, -1.0, -656.73] {
            let mut p = pool();
            p.current_price_a_in_b = Some(value);
            assert!(matches!(
                p.validate(),
                Err(PoolValidationError::PriceOutOfRange { .. })
            ));
        }
    }

    #[test]
    fn nan_price_change_is_rejected() {
        // A NaN here scores maximum stability rather than being ignored,
        // because `(pct.abs() * 100.0) as u64` casts NaN to zero.
        let mut p = pool();
        p.price_change_percent_24h = Some(f64::NAN);
        assert_eq!(
            p.validate(),
            Err(PoolValidationError::NonFiniteField {
                field: "price_change_percent_24h"
            })
        );
    }

    #[test]
    fn unbounded_fees_are_rejected() {
        let mut p = pool();
        p.host_fee_bps = u32::MAX;
        assert!(matches!(
            p.validate(),
            Err(PoolValidationError::FeeOutOfRange { .. })
        ));

        let mut p = pool();
        p.host_fee_bps = 6_000;
        p.lp_fee_bps = 5_000;
        assert!(matches!(
            p.validate(),
            Err(PoolValidationError::FeeOutOfRange { .. })
        ));
    }

    #[test]
    fn live_fee_levels_are_accepted() {
        // Observed on mainnet: 100+200, 50+30, 0+5, 0+20.
        for (host, lp) in [(100, 200), (50, 30), (0, 5), (0, 20)] {
            let mut p = pool();
            p.host_fee_bps = host;
            p.lp_fee_bps = lp;
            assert!(p.validate().is_ok(), "rejected live fees {host}/{lp}");
        }
    }

    #[test]
    fn non_finite_values_survive_deserialization_and_are_caught_by_validate() {
        // DisplayFromStr hands these straight to f64::from_str.
        for literal in ["NaN", "inf", "-inf", "1e999"] {
            let json = format!(
                r#"{{"lpPublicKey":"02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
                     "hostName":"flashnet","hostFeeBps":0,"lpFeeBps":20,
                     "assetAAddress":"{BTC_ASSET_ADDRESS}","assetBAddress":"{USDB}",
                     "currentPriceAInB":"{literal}",
                     "createdAt":"2025-09-22","updatedAt":"2026-08-04"}}"#
            );
            let parsed: Pool = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("{literal} should deserialize, got {e}"));
            assert!(
                parsed.validate().is_err(),
                "{literal} deserialized but was not rejected"
            );
        }
    }

    #[test]
    fn validate_for_swap_accepts_either_direction() {
        let p = pool();
        assert!(
            p.validate_for_swap(BTC_ASSET_ADDRESS, USDB, Network::Mainnet)
                .is_ok()
        );
        assert!(
            p.validate_for_swap(USDB, BTC_ASSET_ADDRESS, Network::Mainnet)
                .is_ok()
        );
    }

    #[test]
    fn validate_for_swap_rejects_an_asset_the_pool_does_not_trade() {
        let other = "1111111111111111111111111111111111111111111111111111111111111111";
        let err = pool()
            .validate_for_swap(other, BTC_ASSET_ADDRESS, Network::Mainnet)
            .unwrap_err();
        assert!(err.to_string().contains("does not trade the pair"));
    }

    #[test]
    fn validate_for_swap_rejects_a_pair_the_pool_only_half_matches() {
        let other = "1111111111111111111111111111111111111111111111111111111111111111";
        assert!(
            pool()
                .validate_for_swap(BTC_ASSET_ADDRESS, other, Network::Mainnet)
                .is_err()
        );
    }

    #[test]
    fn price_matches_reserves_on_a_live_constant_product_pool() {
        // 171239613 / 260745 = 656.7321 against an advertised 656.7331.
        assert_eq!(pool().price_matches_reserves(10), Some(true));
    }

    #[test]
    fn price_matches_reserves_catches_a_deflated_price() {
        let mut p = pool();
        p.current_price_a_in_b = Some(1e-6);
        assert_eq!(p.price_matches_reserves(10), Some(false));
    }

    #[test]
    fn price_matches_reserves_declines_to_judge_a_v3_pool() {
        // The live V3 pool sits 47% apart and is perfectly healthy.
        let mut p = pool();
        p.curve_type = Some(CurveType::V3Concentrated);
        p.asset_a_reserve = Some(124_628_012);
        p.asset_b_reserve = Some(117_572_755_385);
        p.current_price_a_in_b = Some(641.920_746_581_909_7);
        assert_eq!(p.price_matches_reserves(10), None);
    }

    /// Verbatim mainnet `/v1/pools` entries for spark <-> USDB, one per
    /// distinct shape in the listing: zero reserves with a price, zero reserves
    /// with a null price (6 of the 9 live pools), the V3 pool including the
    /// tick fields `Pool` does not model, and the constant-product pool.
    const LIVE_LISTING: &str = r#"{"pools":[
      {"lpPublicKey":"0202e9c857e89901e7211897cc0e69be843f865de845f60589614a693f3c966cef",
       "hostName":"luminex-lite","hostFeeBps":100,"lpFeeBps":200,
       "assetAAddress":"3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
       "assetBAddress":"020202020202020202020202020202020202020202020202020202020202020202",
       "assetAReserve":"0","assetBReserve":"0","currentPriceAInB":"0.004145404739458050",
       "tvlAssetB":"0","volume24hAssetB":"0","priceChangePercent24h":"0.00",
       "curveType":"CONSTANT_PRODUCT","createdAt":"2026-05-10","updatedAt":"2026-05-11"},
      {"lpPublicKey":"03c6923e64efbe90dafd88a50027a00587b960b4829ab52f403fcac69ad47c4065",
       "hostName":"sparksat","hostFeeBps":50,"lpFeeBps":30,
       "assetAAddress":"3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
       "assetBAddress":"020202020202020202020202020202020202020202020202020202020202020202",
       "assetAReserve":"0","assetBReserve":"0","currentPriceAInB":null,
       "tvlAssetB":"0","volume24hAssetB":"0","priceChangePercent24h":"0.00",
       "curveType":"CONSTANT_PRODUCT","createdAt":"2026-04-17","updatedAt":"2026-04-17"},
      {"lpPublicKey":"037579aee81891fc3a28cbe7b13e565031d46248b420fb39dcb308598f472b4513",
       "hostName":"flashnet","hostFeeBps":0,"lpFeeBps":5,
       "assetAAddress":"020202020202020202020202020202020202020202020202020202020202020202",
       "assetBAddress":"3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
       "assetAReserve":"124628012","assetBReserve":"117572755385",
       "currentPriceAInB":"641.920746581909683704","tvlAssetB":"197574061893",
       "volume24hAssetB":"57830801247504","priceChangePercent24h":"1.06",
       "curveType":"V3_CONCENTRATED","createdAt":"2026-02-08","updatedAt":"2026-08-05",
       "currentTick":64647,"tickSpacing":10,"totalLiquidity":"5150839635149"},
      {"lpPublicKey":"02894808873b896e21d29856a6d7bb346fb13c019739adb9bf0b6a8b7e28da53da",
       "hostName":"flashnet","hostFeeBps":0,"lpFeeBps":20,
       "assetAAddress":"020202020202020202020202020202020202020202020202020202020202020202",
       "assetBAddress":"3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca",
       "assetAReserve":"260745","assetBReserve":"171239613",
       "currentPriceAInB":"656.733136003420639891","tvlAssetB":"342479495",
       "volume24hAssetB":"4041946","priceChangePercent24h":"-2.53",
       "curveType":"CONSTANT_PRODUCT","createdAt":"2025-09-22","updatedAt":"2026-08-04"}
     ],"totalCount":4}"#;

    #[test]
    fn the_live_mainnet_listing_validates_in_full() {
        let listing: crate::ListPoolsResponse = serde_json::from_str(LIVE_LISTING).unwrap();
        assert_eq!(listing.pools.len(), 4);
        for p in &listing.pools {
            assert!(
                p.validate().is_ok(),
                "rejected live pool {}: {:?}",
                p.lp_public_key,
                p.validate()
            );
        }
    }

    #[test]
    fn the_live_v3_pool_is_skipped_by_the_reserve_cross_check() {
        // It is the pool production selects, and its reserve ratio sits 47%
        // from the advertised price, so judging it would reject every swap.
        let listing: crate::ListPoolsResponse = serde_json::from_str(LIVE_LISTING).unwrap();
        let v3 = listing
            .pools
            .iter()
            .find(|p| p.curve_type == Some(CurveType::V3Concentrated))
            .unwrap();
        assert_eq!(v3.price_matches_reserves(10), None);
        // The tick is the check that does apply to it, and it comes from the
        // same verbatim payload rather than a hand-built fixture.
        assert_eq!(v3.current_tick, Some(64_647));
        assert_eq!(v3.price_matches_tick(10), Some(true));

        let cp = listing
            .pools
            .iter()
            .find(|p| p.asset_a_reserve == Some(260_745))
            .unwrap();
        assert_eq!(cp.price_matches_reserves(10), Some(true));
    }

    #[test]
    fn the_live_listing_is_swappable_in_both_directions() {
        let listing: crate::ListPoolsResponse = serde_json::from_str(LIVE_LISTING).unwrap();
        for p in &listing.pools {
            let (a, b) = (p.asset_a_address.clone(), p.asset_b_address.clone());
            assert!(p.validate_for_swap(&a, &b, Network::Mainnet).is_ok());
            assert!(p.validate_for_swap(&b, &a, Network::Mainnet).is_ok());
        }
    }

    /// The live V3 pool, whose tick and price are both real.
    fn v3_pool() -> Pool {
        let mut p = pool();
        p.curve_type = Some(CurveType::V3Concentrated);
        p.asset_a_reserve = Some(124_628_012);
        p.asset_b_reserve = Some(117_572_755_385);
        p.current_price_a_in_b = Some(641.920_746_581_909_7);
        p.current_tick = Some(64_647);
        p.tick_spacing = Some(10);
        p.total_liquidity = Some(5_150_839_635_149);
        p
    }

    #[test]
    fn declaring_concentrated_liquidity_without_a_tick_is_rejected() {
        // The claim alone wins selection priority and skips the reserve maths,
        // so it has to be backed by the tick that encodes the price.
        let mut p = v3_pool();
        p.current_tick = None;
        assert_eq!(p.validate(), Err(PoolValidationError::UncorroboratedCurve));
    }

    #[test]
    fn declaring_concentrated_liquidity_with_a_contradicting_tick_is_rejected() {
        let mut p = v3_pool();
        p.current_price_a_in_b = Some(1e-6);
        assert_eq!(p.validate(), Err(PoolValidationError::UncorroboratedCurve));
    }

    #[test]
    fn a_genuine_concentrated_liquidity_pool_validates() {
        assert!(v3_pool().validate().is_ok());
    }

    #[test]
    fn a_price_contradicting_the_reserves_is_rejected() {
        let mut p = pool();
        p.current_price_a_in_b = Some(1e-6);
        assert_eq!(
            p.validate(),
            Err(PoolValidationError::PriceContradictsReserves)
        );
    }

    #[test]
    fn renaming_the_curve_does_not_lift_validation() {
        // Naming a curve selects that curve's identity, never an exemption. A
        // rogue entry inflating a reserve while leaving the price honest is
        // caught under constant product...
        let mut p = pool();
        p.asset_b_reserve = p.asset_b_reserve.map(|r| r * 10_000);
        assert!(matches!(
            p.validate(),
            Err(PoolValidationError::PriceContradictsReserves)
        ));

        // ...and a curve this client cannot price is refused both a quote and
        // validation, so renaming buys unquotable rather than unchecked. Being
        // refused by both is what keeps such a pool counted as rejected instead
        // of reported as missing liquidity.
        p.asset_b_reserve = pool().asset_b_reserve;
        for curve in [Some(CurveType::Unknown), None] {
            p.curve_type.clone_from(&curve);
            assert!(
                matches!(p.validate(), Err(PoolValidationError::UnsupportedCurve)),
                "validated a pool with curve {curve:?}"
            );
            assert!(
                p.calculate_amount_in(BTC_ASSET_ADDRESS, 1_000, 10, 0, Network::Mainnet)
                    .is_err(),
                "quoted a pool with curve {curve:?}"
            );
        }
    }

    /// Verbatim mainnet `/v1/pools` single-sided entries, 2026-08-08: one at the
    /// start of its curve and one part way along it. `LIVE_LISTING` has neither,
    /// and these carry the bonding fields plus a price in capital-E scientific
    /// notation, none of which any other fixture exercises.
    const LIVE_BONDING: &str = r#"{"pools":[
      {"lpPublicKey":"02be3c07f047b01955c52fc522b592273f8b4bb3854ee8653bb3f54a71f6d4c056",
       "hostName":"luminex","hostFeeBps":100,"lpFeeBps":30,
       "assetAAddress":"5a8b7b3b68e4c40370c588f884d20379656f63e57ba097540c8867a5132140bf",
       "assetBAddress":"020202020202020202020202020202020202020202020202020202020202020202",
       "assetAReserve":"210000000000000000","assetBReserve":"0",
       "currentPriceAInB":"0.008333333333333330","tvlAssetB":"1749999999999999",
       "volume24hAssetB":"0","priceChangePercent24h":"0.00","curveType":"SINGLE_SIDED",
       "createdAt":"2025-10-28 22:20:45.736702 +00:00:00",
       "updatedAt":"2026-02-24 13:30:20.459361 +00:00:00",
       "virtualReserveA":"224000000000000000","virtualReserveB":"1866666666666666",
       "thresholdPct":80,"initialReserveA":"210000000000000000",
       "bondingProgressPercent":"0.0000","graduationThresholdAmount":"168000000000000000"},
      {"lpPublicKey":"03f33d87a373b7a83ff4998a05816e39d2a413334e4000858bf2ad2de3849e5896",
       "hostName":"luminex-lite","hostFeeBps":100,"lpFeeBps":30,
       "assetAAddress":"d250c86c85cd5286c2687c04081bedb6b02369ea81284049708176dcd1ab1cc7",
       "assetBAddress":"020202020202020202020202020202020202020202020202020202020202020202",
       "assetAReserve":"59993557188021504","assetBReserve":"13722",
       "currentPriceAInB":"2.129883741317975937E-9","tvlAssetB":"127793024",
       "volume24hAssetB":"0","priceChangePercent24h":"0.00","curveType":"SINGLE_SIDED",
       "createdAt":"2025-12-04 14:10:08.097381 +00:00:00",
       "updatedAt":"2026-08-06 13:32:09.012587 +00:00:00",
       "virtualReserveA":"108000000000000000","virtualReserveB":"230000000",
       "thresholdPct":60,"initialReserveA":"60000000000000000",
       "bondingProgressPercent":"0.0179","graduationThresholdAmount":"36000000000000000"}
    ],"totalCount":2}"#;

    fn live_bonding_pools() -> Vec<Pool> {
        let listing: crate::ListPoolsResponse =
            serde_json::from_str(LIVE_BONDING).expect("live bonding listing deserializes");
        listing.pools
    }

    #[test]
    fn the_live_bonding_fields_deserialize() {
        // They are `DisplayFromStr` over `Option<u128>`: a wire-format change
        // leaves them `None`, which makes every such pool silently unquotable
        // rather than failing loudly.
        for p in live_bonding_pools() {
            assert!(p.virtual_reserve_a.is_some(), "virtual_reserve_a");
            assert!(p.virtual_reserve_b.is_some(), "virtual_reserve_b");
            assert!(p.initial_reserve_a.is_some(), "initial_reserve_a");
            assert!(p.current_price_a_in_b.is_some(), "price");
            assert_eq!(p.curve_type, Some(CurveType::SingleSided));
            assert!(p.quote_reserves().is_some(), "quote_reserves");
        }
    }

    #[test]
    fn every_live_bonding_pool_corroborates_its_own_price() {
        // One at progress 0, where the curve has not moved, and one part way
        // along it. Both reproduce the advertised price from their virtual pair.
        for p in live_bonding_pools() {
            assert_eq!(
                p.price_matches_reserves(RESERVE_TOLERANCE_BPS),
                Some(true),
                "pool {} was not corroborated",
                p.lp_public_key
            );
        }
    }

    #[test]
    fn a_bonding_curve_missing_its_fields_is_refused_not_repriced() {
        // Without the virtual pair there is nothing to price the curve on, and
        // the advertised price is bounded only by the range check. Quoting off
        // it would reopen by omission the escape that naming a curve closed.
        let mut bonding = live_bonding_pools()[1].clone();
        bonding.virtual_reserve_b = None;

        assert!(bonding.quote_reserves().is_none());
        assert_eq!(bonding.price_matches_reserves(RESERVE_TOLERANCE_BPS), None);
        assert!(
            bonding
                .calculate_amount_in(
                    BTC_ASSET_ADDRESS,
                    1_000_000_000_000,
                    10,
                    0,
                    Network::Mainnet
                )
                .is_err(),
            "quoted a bonding curve with no curve to quote against"
        );

        // The same holds for a constant-product pool with no reserves.
        let mut constant_product = pool();
        constant_product.asset_b_reserve = None;
        assert!(
            constant_product
                .calculate_amount_in(BTC_ASSET_ADDRESS, 1_000, 10, 0, Network::Mainnet)
                .is_err()
        );
    }

    #[test]
    fn a_bonding_curve_cannot_pay_out_more_than_it_holds() {
        // The virtual pair it prices against runs far above its balance: this
        // pool holds 13,722 sats and prices B against 230,013,721. Half the
        // priced figure clears the consumption cap and the pool still cannot
        // pay it.
        let pool = &live_bonding_pools()[1];
        let held = pool.asset_b_reserve.expect("a balance");
        let asset_a = pool.asset_a_address.clone();

        assert!(
            pool.calculate_amount_in(&asset_a, held / 2, 10, 0, Network::Mainnet)
                .is_ok(),
            "refused a draw the pool can cover"
        );
        let err = pool
            .calculate_amount_in(&asset_a, held, 10, 0, Network::Mainnet)
            .expect_err("quoted more than the pool holds");
        assert!(
            err.to_string().contains("more than the pool holds"),
            "{err}"
        );
    }

    #[test]
    fn no_quote_promises_more_than_a_pool_holds() {
        // The property the curve fixtures did not assert: a quote is only worth
        // making if the pool can settle it. Swept across both directions and a
        // range of sizes, including ones the consumption cap lets through.
        let listing: crate::ListPoolsResponse = serde_json::from_str(LIVE_LISTING).unwrap();
        for pool in live_bonding_pools().iter().chain(listing.pools.iter()) {
            for (asset_in, held_out) in [
                (&pool.asset_a_address, pool.asset_b_reserve),
                (&pool.asset_b_address, pool.asset_a_reserve),
            ] {
                let Some(held) = held_out else { continue };
                for amount_out in [1u128, held / 2, held, held * 2, held * 1_000] {
                    if pool
                        .calculate_amount_in(asset_in, amount_out, 10, 0, Network::Mainnet)
                        .is_ok()
                    {
                        assert!(
                            amount_out < held,
                            "quoted {amount_out} against a balance of {held}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_bonding_curve_is_quoted_off_its_curve_not_its_reserves() {
        // Paying BTC for 1e12 of the token. The curve prices that near
        // `price * amount_out`, about 2,130 sats. The real reserves hold only
        // 13,722 sats against 6e16 of the token, so quoting off them returns
        // single-digit sats: a roughly 2,000x underpayment, which is the harm
        // #27 describes.
        let pool = &live_bonding_pools()[1];
        let amount_out = 1_000_000_000_000u128;
        let amount_in = pool
            .calculate_amount_in(BTC_ASSET_ADDRESS, amount_out, 10, 0, Network::Mainnet)
            .expect("quotable");

        #[allow(clippy::cast_precision_loss)]
        let expected = pool.current_price_a_in_b.unwrap() * amount_out as f64;
        #[allow(clippy::cast_precision_loss)]
        let ratio = amount_in as f64 / expected;
        assert!(
            (1.0..1.2).contains(&ratio),
            "quote {amount_in} is {ratio}x the curve price, expected near 1"
        );
    }

    #[test]
    fn a_bonding_curve_is_judged_by_its_virtual_reserves() {
        // Mainnet, 2026-08-08. Its real reserves imply 2.29e-13 against an
        // advertised 2.13e-09, four orders out, because a single-sided pool
        // does not price off them. The traversed virtual pair reproduces the
        // advertised price to under a basis point.
        let mut p = pool();
        p.curve_type = Some(CurveType::SingleSided);
        p.virtual_reserve_a = Some(108_000_000_000_000_000);
        p.virtual_reserve_b = Some(230_000_000);
        p.initial_reserve_a = Some(60_000_000_000_000_000);
        p.asset_a_reserve = Some(59_993_557_188_021_504);
        p.asset_b_reserve = Some(13_722);
        p.current_price_a_in_b = Some(2.129_883_741_317_976e-9);
        assert_eq!(p.price_matches_reserves(RESERVE_TOLERANCE_BPS), Some(true));

        // And it still catches a price the curve does not support.
        p.current_price_a_in_b = Some(2.129_883_741_317_976e-6);
        assert_eq!(p.price_matches_reserves(RESERVE_TOLERANCE_BPS), Some(false));
    }

    #[test]
    fn a_tick_matching_either_orientation_of_the_pair_is_accepted() {
        // Pools do not order their pair consistently: the live listing carries
        // both. A tick encoding the inverse price is still corroboration.
        let mut p = v3_pool();
        p.current_price_a_in_b = Some(1.0 / 641.920_746_581_909_7);
        assert_eq!(p.price_matches_tick(TICK_TOLERANCE_BPS), Some(true));
        assert!(p.validate().is_ok());
    }

    #[test]
    fn the_tick_tolerance_leaves_room_around_the_live_data() {
        // A rejected concentrated pool drops the pair back to constant product,
        // which carries the reserve-draw cap. The live pool sits 0.89 bps from
        // its tick, so 40 has to pass.
        let mut near = v3_pool();
        near.current_price_a_in_b = Some(641.864_220 * 1.004);
        assert_eq!(near.price_matches_tick(TICK_TOLERANCE_BPS), Some(true));
        assert!(near.validate().is_ok());

        // And it still has to mean something.
        let mut far = v3_pool();
        far.current_price_a_in_b = Some(641.864_220 * 1.05);
        assert_eq!(far.price_matches_tick(TICK_TOLERANCE_BPS), Some(false));
    }

    #[test]
    fn the_reserve_tolerance_leaves_room_for_unsynchronised_sampling() {
        // Reserves and price need not come from the same instant.
        let mut near = pool();
        near.current_price_a_in_b = Some(656.732_106 * 1.02);
        assert_eq!(
            near.price_matches_reserves(RESERVE_TOLERANCE_BPS),
            Some(true)
        );

        let mut far = pool();
        far.current_price_a_in_b = Some(656.732_106 * 1.1);
        assert_eq!(
            far.price_matches_reserves(RESERVE_TOLERANCE_BPS),
            Some(false)
        );
    }

    #[test]
    fn an_inflated_price_contradicts_the_reserves_too() {
        // The tolerance is applied to the advertised price, so the accepted
        // band is asymmetric. Both directions have to reject a gross mismatch.
        let mut low = pool();
        low.current_price_a_in_b = Some(656.733_136 / 4.0);
        assert_eq!(
            low.price_matches_reserves(RESERVE_TOLERANCE_BPS),
            Some(false)
        );

        let mut high = pool();
        high.current_price_a_in_b = Some(656.733_136 * 4.0);
        assert_eq!(
            high.price_matches_reserves(RESERVE_TOLERANCE_BPS),
            Some(false)
        );
    }

    #[test]
    fn a_tiny_price_from_token_decimals_is_not_rejected() {
        // Base-unit pricing means an 18-decimal token is legitimately minute.
        // Reserves cleared so the price bound is what is under test: this
        // fixture's reserves imply 656.73 and would contradict the price.
        let mut p = pool();
        p.asset_a_reserve = None;
        p.asset_b_reserve = None;
        p.current_price_a_in_b = Some(1e-18);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn the_live_v3_tick_agrees_with_its_advertised_price() {
        // 1.0001^64647 = 641.864 against an advertised 641.9207, 0.9 bps apart.
        // One tick is 1 bp and one spacing step is 10 bps, so allow a spacing.
        assert_eq!(v3_pool().price_matches_tick(10), Some(true));
    }

    #[test]
    fn price_matches_tick_catches_a_price_the_tick_contradicts() {
        let mut p = v3_pool();
        p.current_price_a_in_b = Some(1e-6);
        assert_eq!(p.price_matches_tick(10), Some(false));
    }

    #[test]
    fn price_matches_tick_declines_without_a_tick() {
        // Only V3 pools carry one, which is why this is the check for them.
        assert_eq!(pool().price_matches_tick(10), None);
    }

    #[test]
    fn the_two_cross_checks_never_both_apply() {
        for p in [pool(), v3_pool()] {
            assert!(
                p.price_matches_reserves(10).is_none() || p.price_matches_tick(10).is_none(),
                "both checks judged the same pool"
            );
        }
    }

    #[test]
    fn price_matches_reserves_declines_without_data() {
        let mut p = pool();
        p.asset_a_reserve = Some(0);
        assert_eq!(p.price_matches_reserves(10), None);

        let mut p = pool();
        p.current_price_a_in_b = None;
        assert_eq!(p.price_matches_reserves(10), None);
    }
}
