//! Conversion-related helpers: enriching [`Payment`]s with [`ConversionDetails`]
//! from child payments and cross-chain `ConversionInfo`, and building the
//! per-step [`Conversion`] entries that populate the public API.
//!
//! The public types live in [`crate::models`]; this module hosts the free-fn
//! builders, enrichment, and status-folding logic consumed by
//! `sdk::payments::{list_payments, get_payment, send_payment}` and the
//! cross-chain event listeners.

use std::sync::Arc;

use tracing::warn;

use crate::{
    ConversionInfo, ConversionStatus, Payment, PaymentDetails, PaymentType, Storage,
    error::SdkError,
    models::{
        Conversion, ConversionAsset, ConversionChain, ConversionDetails, ConversionProvider,
        ConversionSide,
    },
};

/// Extract `ConversionInfo` from whichever [`PaymentDetails`] variant carries
/// it. Cross-chain conversion info can sit on `Lightning` (Boltz hold-invoice
/// pays), `Spark`, or `Token` details — this helper hides the variant match
/// so callers can write a single destructure regardless of provider.
pub(crate) fn extract_conversion_info(details: Option<PaymentDetails>) -> Option<ConversionInfo> {
    match details? {
        PaymentDetails::Spark {
            conversion_info, ..
        }
        | PaymentDetails::Token {
            conversion_info, ..
        }
        | PaymentDetails::Lightning {
            conversion_info, ..
        } => conversion_info,
        _ => None,
    }
}

/// Gets a payment from storage by ID to include already stored payment metadata
/// and then enriches it with conversions by looking up related child payments
/// and the payment's own conversion info.
pub async fn get_payment_with_conversion_details(
    id: String,
    storage: Arc<dyn Storage>,
) -> Result<Payment, SdkError> {
    let mut payment = storage.get_payment_by_id(id).await?;

    if needs_enrichment(&payment) {
        let children = if payment.conversion_details.is_some() {
            storage
                .get_payments_by_parent_ids(vec![payment.id.clone()])
                .await?
                .remove(&payment.id)
        } else {
            None
        };
        enrich_payment(&mut payment, children.as_deref());
    }

    Ok(payment)
}

/// Whether a payment requires conversion enrichment — true if it carries
/// `ConversionDetails` (AMM / stable-balance case) or cross-chain
/// `ConversionInfo` (Orchestra / Boltz).
pub(crate) fn needs_enrichment(payment: &Payment) -> bool {
    payment.conversion_details.is_some()
        || extract_conversion_info(payment.details.clone())
            .is_some_and(|info| !matches!(info, ConversionInfo::Amm { .. }))
}

/// Populates `payment.conversion_details` with the ordered per-step
/// [`Conversion`]s and a folded overall status. No-op if the payment does
/// not need enrichment.
///
/// Pure function: callers are responsible for supplying `child_payments`
/// (either from a prefetched parent→children map in `list_payments`, or
/// from a single-parent lookup in `get_payment_with_conversion_details`).
pub(crate) fn enrich_payment(payment: &mut Payment, child_payments: Option<&[Payment]>) {
    if !needs_enrichment(payment) {
        return;
    }

    let conversions = build_conversions(payment, child_payments);
    if conversions.is_empty() {
        return;
    }

    let overall_status = fold_conversion_status(&conversions);
    match payment.conversion_details.as_mut() {
        Some(cd) => {
            cd.conversions = conversions;
            cd.status = overall_status;
        }
        None => {
            payment.conversion_details = Some(ConversionDetails {
                status: overall_status,
                conversions,
            });
        }
    }
}

/// Folds per-step statuses into a single overall status. Priority:
/// `Failed` > `RefundNeeded` > `Pending` > `Refunded` > `Completed`.
///
/// Rationale: any hard failure dominates; any pending step keeps the overall
/// pending; `Refunded` is a terminal success-for-recovery but not a true
/// completion, so it sits between pending and completed.
fn fold_conversion_status(conversions: &[Conversion]) -> ConversionStatus {
    fn rank(s: &ConversionStatus) -> u8 {
        match s {
            ConversionStatus::Failed => 4,
            ConversionStatus::RefundNeeded => 3,
            ConversionStatus::Pending => 2,
            ConversionStatus::Refunded => 1,
            ConversionStatus::Completed => 0,
        }
    }

    conversions
        .iter()
        .map(|c| c.status.clone())
        .max_by_key(rank)
        .unwrap_or(ConversionStatus::Completed)
}

/// Builds the ordered list of conversions for a payment from its child payments
/// and its own conversion info.
///
/// - AMM conversions are built from send/receive child payment pairs
/// - Cross-chain conversions are built from Orchestra/Boltz `ConversionInfo` on the parent
/// - Ordering is directional: Send = [AMM, cross-chain], Receive = [cross-chain, AMM]
pub(crate) fn build_conversions(
    payment: &Payment,
    child_payments: Option<&[Payment]>,
) -> Vec<Conversion> {
    let mut amm_conversion = None;
    let mut crosschain_conversion = None;

    // Build AMM conversion from child payments.
    // For ongoing sends: both send+receive children exist.
    // For auto-conversions: only send child exists; the parent IS the receive side.
    if let Some(children) = child_payments {
        let send = children
            .iter()
            .find(|p| p.payment_type == PaymentType::Send);
        let recv = children
            .iter()
            .find(|p| p.payment_type == PaymentType::Receive);

        let pair = match (send, recv) {
            (Some(s), Some(r)) => Some((s, r)),
            (Some(s), None) => Some((s, payment)),
            (None, Some(r)) => Some((payment, r)),
            (None, None) => None,
        };

        if let Some((s, r)) = pair {
            match build_amm_conversion(s, r) {
                Ok(conv) => amm_conversion = Some(conv),
                Err(e) => warn!("Failed to build AMM conversion: {e}"),
            }
        }
    }

    // Build cross-chain conversion from parent's own ConversionInfo
    if let Some(info) = extract_conversion_info(payment.details.clone()) {
        crosschain_conversion = build_crosschain_conversion(&info, payment);
    }

    // Order directionally
    let mut conversions = Vec::new();
    match payment.payment_type {
        PaymentType::Send => {
            conversions.extend(amm_conversion);
            conversions.extend(crosschain_conversion);
        }
        PaymentType::Receive => {
            conversions.extend(crosschain_conversion);
            conversions.extend(amm_conversion);
        }
    }
    conversions
}

/// Components extracted from a payment's details for building a conversion side.
struct SideInfo<'a> {
    chain: ConversionChain,
    asset: ConversionAsset,
    conversion_info: Option<&'a ConversionInfo>,
}

/// Extracts chain, asset, and conversion info from a payment's details.
fn extract_side_info(payment: &Payment) -> Result<SideInfo<'_>, SdkError> {
    match &payment.details {
        Some(PaymentDetails::Token {
            metadata,
            conversion_info,
            ..
        }) => Ok(SideInfo {
            chain: ConversionChain::Spark,
            asset: ConversionAsset {
                ticker: metadata.ticker.clone(),
                identifier: Some(metadata.identifier.clone()),
                decimals: metadata.decimals,
            },
            conversion_info: conversion_info.as_ref(),
        }),
        Some(PaymentDetails::Spark {
            conversion_info, ..
        }) => Ok(SideInfo {
            chain: ConversionChain::Spark,
            asset: btc_asset(),
            conversion_info: conversion_info.as_ref(),
        }),
        Some(PaymentDetails::Lightning {
            conversion_info, ..
        }) => Ok(SideInfo {
            chain: ConversionChain::Lightning,
            asset: btc_asset(),
            conversion_info: conversion_info.as_ref(),
        }),
        _ => Err(SdkError::Generic(format!(
            "Unsupported payment details for conversion side on payment {}",
            payment.id
        ))),
    }
}

/// The BTC/sats asset — amounts are already in the smallest unit.
fn btc_asset() -> ConversionAsset {
    ConversionAsset {
        ticker: "BTC".to_string(),
        identifier: None,
        decimals: 0,
    }
}

/// Builds an AMM conversion from a send/receive child payment pair.
pub fn build_amm_conversion(send: &Payment, recv: &Payment) -> Result<Conversion, SdkError> {
    let from_side = extract_side_info(send)?;
    let to_side = extract_side_info(recv)?;

    let from_conv_fee = from_side
        .conversion_info
        .and_then(ConversionInfo::fee)
        .unwrap_or(0);
    let to_conv_fee = to_side
        .conversion_info
        .and_then(ConversionInfo::fee)
        .unwrap_or(0);

    let amm_info = from_side
        .conversion_info
        .filter(|i| i.is_amm())
        .or_else(|| to_side.conversion_info.filter(|i| i.is_amm()));
    let (status, amount_adjustment) = match amm_info {
        Some(ConversionInfo::Amm {
            status,
            amount_adjustment,
            ..
        }) => (status.clone(), amount_adjustment.clone()),
        _ => (ConversionStatus::Completed, None),
    };

    Ok(Conversion {
        provider: ConversionProvider::Amm,
        status,
        from: ConversionSide {
            chain: from_side.chain,
            asset: from_side.asset,
            amount: send.amount,
            fee: send.fees.saturating_add(from_conv_fee),
        },
        to: ConversionSide {
            chain: to_side.chain,
            asset: to_side.asset,
            amount: recv.amount,
            fee: recv.fees.saturating_add(to_conv_fee),
        },
        amount_adjustment,
    })
}

/// Builds a cross-chain conversion from an Orchestra or Boltz `ConversionInfo`.
/// Returns None for AMM conversion info (handled separately via child payments).
pub fn build_crosschain_conversion(
    info: &ConversionInfo,
    source_payment: &Payment,
) -> Option<Conversion> {
    let from_side = extract_side_info(source_payment).ok()?;

    match info {
        ConversionInfo::Orchestra {
            chain,
            chain_id,
            asset,
            estimated_out,
            delivered_amount,
            status,
            fee,
            asset_decimals,
            asset_contract,
            ..
        } => Some(Conversion {
            provider: ConversionProvider::Orchestra,
            status: status.clone(),
            from: ConversionSide {
                chain: from_side.chain,
                asset: from_side.asset,
                amount: source_payment.amount,
                fee: 0,
            },
            to: ConversionSide {
                chain: ConversionChain::External {
                    name: chain.clone(),
                    chain_id: chain_id.clone(),
                },
                asset: ConversionAsset {
                    ticker: asset.clone(),
                    identifier: asset_contract.clone(),
                    decimals: *asset_decimals,
                },
                amount: delivered_amount.unwrap_or(*estimated_out),
                fee: fee.unwrap_or(0),
            },
            amount_adjustment: None,
        }),
        ConversionInfo::Boltz {
            chain,
            chain_id,
            asset,
            invoice_amount_sats,
            estimated_out,
            delivered_amount,
            status,
            fee,
            asset_decimals,
            asset_contract,
            ..
        } => Some(Conversion {
            provider: ConversionProvider::Boltz,
            status: status.clone(),
            from: ConversionSide {
                chain: from_side.chain,
                asset: from_side.asset,
                amount: u128::from(*invoice_amount_sats),
                fee: fee.unwrap_or(0),
            },
            to: ConversionSide {
                chain: ConversionChain::External {
                    name: chain.clone(),
                    chain_id: chain_id.clone(),
                },
                asset: ConversionAsset {
                    ticker: asset.clone(),
                    identifier: asset_contract.clone(),
                    decimals: *asset_decimals,
                },
                amount: delivered_amount.unwrap_or(*estimated_out),
                fee: 0,
            },
            amount_adjustment: None,
        }),
        ConversionInfo::Amm { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AmountAdjustmentReason, SparkHtlcDetails, SparkHtlcStatus,
        models::{PaymentMethod, PaymentStatus, TokenMetadata, TokenTransactionType},
    };

    fn test_token_metadata() -> TokenMetadata {
        TokenMetadata {
            identifier: "token123".to_string(),
            issuer_public_key: "02abcdef".to_string(),
            name: "USD Balance".to_string(),
            ticker: "USDB".to_string(),
            decimals: 6,
            max_supply: 21_000_000,
            is_freezable: false,
        }
    }

    fn amm_info(status: ConversionStatus, fee: u128) -> ConversionInfo {
        ConversionInfo::Amm {
            pool_id: "pool_1".to_string(),
            conversion_id: "conv_1".to_string(),
            status,
            fee: Some(fee),
            purpose: None,
            amount_adjustment: None,
        }
    }

    fn amm_info_with_adjustment(adjustment: AmountAdjustmentReason) -> ConversionInfo {
        ConversionInfo::Amm {
            pool_id: "pool_1".to_string(),
            conversion_id: "conv_1".to_string(),
            status: ConversionStatus::Completed,
            fee: Some(10),
            purpose: None,
            amount_adjustment: Some(adjustment),
        }
    }

    fn test_htlc_details() -> SparkHtlcDetails {
        SparkHtlcDetails {
            payment_hash: "hash123".to_string(),
            preimage: None,
            expiry_time: 0,
            status: SparkHtlcStatus::PreimageShared,
        }
    }

    fn token_payment(
        id: &str,
        ptype: PaymentType,
        amount: u128,
        fees: u128,
        info: ConversionInfo,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: ptype,
            status: PaymentStatus::Completed,
            amount,
            fees,
            timestamp: 1000,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: test_token_metadata(),
                tx_hash: "tx_1".to_string(),
                tx_type: TokenTransactionType::Transfer,
                invoice_details: None,
                conversion_info: Some(info),
            }),
            conversion_details: None,
        }
    }

    fn spark_payment(
        id: &str,
        ptype: PaymentType,
        amount: u128,
        fees: u128,
        info: ConversionInfo,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: ptype,
            status: PaymentStatus::Completed,
            amount,
            fees,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: Some(info),
            }),
            conversion_details: None,
        }
    }

    fn lightning_payment_with_info(
        id: &str,
        amount: u128,
        fees: u128,
        info: ConversionInfo,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount,
            fees,
            timestamp: 1000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: None,
                invoice: "lnbc1000n1p".to_string(),
                destination_pubkey: "02abc".to_string(),
                htlc_details: test_htlc_details(),
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: Some(info),
            }),
            conversion_details: None,
        }
    }

    fn orchestra_info(status: ConversionStatus) -> ConversionInfo {
        ConversionInfo::Orchestra {
            order_id: "ord_1".to_string(),
            quote_id: "q_1".to_string(),
            chain: "base".to_string(),
            chain_id: Some("8453".to_string()),
            asset: "USDC".to_string(),
            recipient_address: "0x1234".to_string(),
            estimated_out: 99_500_000,
            delivered_amount: None,
            status,
            fee: Some(500),
            read_token: None,
            asset_decimals: 6,
            asset_contract: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
        }
    }

    fn boltz_info(delivered: Option<u128>, status: ConversionStatus) -> ConversionInfo {
        ConversionInfo::Boltz {
            swap_id: "swap_1".to_string(),
            chain: "solana".to_string(),
            chain_id: None,
            asset: "USDT".to_string(),
            recipient_address: "So1ana".to_string(),
            invoice: "lnbc1000n1p".to_string(),
            invoice_amount_sats: 100_000,
            estimated_out: 1_450_000,
            delivered_amount: delivered,
            lz_guid: None,
            status,
            fee: Some(1_500),
            max_slippage_bps: 100,
            quote_degraded: false,
            asset_decimals: 6,
            asset_contract: Some("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string()),
        }
    }

    fn parent_send_lightning(info: ConversionInfo) -> Payment {
        Payment {
            id: "parent_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 100_000,
            fees: 3,
            timestamp: 1000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: None,
                invoice: "lnbc1000n1p".to_string(),
                destination_pubkey: "02abc".to_string(),
                htlc_details: test_htlc_details(),
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: Some(info),
            }),
            conversion_details: Some(ConversionDetails {
                status: ConversionStatus::Completed,
                conversions: vec![],
            }),
        }
    }

    fn parent_send_no_crosschain() -> Payment {
        Payment {
            id: "parent_1".to_string(),
            payment_type: PaymentType::Send,
            status: PaymentStatus::Completed,
            amount: 1_500,
            fees: 3,
            timestamp: 1000,
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: None,
                invoice: "lnbc1000n1p".to_string(),
                destination_pubkey: "02abc".to_string(),
                htlc_details: test_htlc_details(),
                lnurl_pay_info: None,
                lnurl_withdraw_info: None,
                lnurl_receive_metadata: None,
                conversion_info: None,
            }),
            conversion_details: Some(ConversionDetails {
                status: ConversionStatus::Completed,
                conversions: vec![],
            }),
        }
    }

    fn parent_receive_no_crosschain() -> Payment {
        Payment {
            id: "parent_1".to_string(),
            payment_type: PaymentType::Receive,
            status: PaymentStatus::Completed,
            amount: 1_500,
            fees: 0,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: None,
            }),
            conversion_details: Some(ConversionDetails {
                status: ConversionStatus::Completed,
                conversions: vec![],
            }),
        }
    }

    // --- build_amm_conversion tests ---

    #[test]
    fn amm_token_to_btc() {
        let send = token_payment(
            "s1",
            PaymentType::Send,
            1_500_000,
            10,
            amm_info(ConversionStatus::Completed, 21),
        );
        let recv = spark_payment(
            "r1",
            PaymentType::Receive,
            1_500,
            0,
            amm_info(ConversionStatus::Completed, 0),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(conv.provider, ConversionProvider::Amm);
        assert_eq!(conv.from.chain, ConversionChain::Spark);
        assert_eq!(conv.from.asset.ticker, "USDB");
        assert_eq!(
            conv.from.asset.identifier,
            Some("token123".to_string()),
            "Token side should carry the Spark token identifier"
        );
        assert_eq!(conv.from.amount, 1_500_000);
        assert_eq!(conv.from.fee, 31);
        assert_eq!(conv.from.asset.decimals, 6);
        assert_eq!(conv.to.chain, ConversionChain::Spark);
        assert_eq!(conv.to.asset.ticker, "BTC");
        assert_eq!(
            conv.to.asset.identifier, None,
            "BTC/sats side should have no identifier"
        );
        assert_eq!(conv.to.amount, 1_500);
        assert_eq!(conv.to.fee, 0);
        assert_eq!(conv.to.asset.decimals, 0);
        assert!(conv.amount_adjustment.is_none());
    }

    #[test]
    fn amm_btc_to_token() {
        let send = spark_payment(
            "s1",
            PaymentType::Send,
            1_500,
            5,
            amm_info(ConversionStatus::Completed, 0),
        );
        let recv = token_payment(
            "r1",
            PaymentType::Receive,
            1_500_000,
            0,
            amm_info(ConversionStatus::Completed, 21),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(conv.from.chain, ConversionChain::Spark);
        assert_eq!(conv.from.asset.ticker, "BTC");
        assert_eq!(conv.from.amount, 1_500);
        assert_eq!(conv.from.fee, 5);
        assert_eq!(conv.from.asset.decimals, 0);
        assert_eq!(conv.to.chain, ConversionChain::Spark);
        assert_eq!(conv.to.asset.ticker, "USDB");
        assert_eq!(conv.to.amount, 1_500_000);
        assert_eq!(conv.to.asset.decimals, 6);
    }

    #[test]
    fn amm_with_amount_adjustment() {
        let send = token_payment(
            "s1",
            PaymentType::Send,
            1_500_000,
            0,
            amm_info_with_adjustment(AmountAdjustmentReason::FlooredToMinLimit),
        );
        let recv = spark_payment(
            "r1",
            PaymentType::Receive,
            1_500,
            0,
            amm_info(ConversionStatus::Completed, 0),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(
            conv.amount_adjustment,
            Some(AmountAdjustmentReason::FlooredToMinLimit)
        );
    }

    #[test]
    fn amm_fees_combined() {
        let send = token_payment(
            "s1",
            PaymentType::Send,
            1_000_000,
            10,
            amm_info(ConversionStatus::Completed, 21),
        );
        let recv = spark_payment(
            "r1",
            PaymentType::Receive,
            1_000,
            5,
            amm_info(ConversionStatus::Completed, 0),
        );

        let conv = build_amm_conversion(&send, &recv).unwrap();
        assert_eq!(conv.from.fee, 31);
        assert_eq!(conv.to.fee, 5);
    }

    // --- build_crosschain_conversion tests ---

    #[test]
    fn orchestra_from_spark() {
        let info = orchestra_info(ConversionStatus::Pending);
        let payment = spark_payment("p1", PaymentType::Send, 100_000, 0, info.clone());

        let conv = build_crosschain_conversion(&info, &payment).unwrap();
        assert_eq!(conv.provider, ConversionProvider::Orchestra);
        assert_eq!(conv.status, ConversionStatus::Pending);
        assert_eq!(conv.from.chain, ConversionChain::Spark);
        assert_eq!(conv.from.asset.ticker, "BTC");
        assert_eq!(conv.from.amount, 100_000);
        assert_eq!(conv.from.fee, 0);
        assert_eq!(
            conv.to.chain,
            ConversionChain::External {
                name: "base".to_string(),
                chain_id: Some("8453".to_string()),
            }
        );
        assert_eq!(conv.to.asset.ticker, "USDC");
        assert_eq!(
            conv.to.asset.identifier,
            Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            "Cross-chain destination should carry the contract address as identifier"
        );
        assert_eq!(conv.to.amount, 99_500_000);
        assert_eq!(conv.to.fee, 500);
        assert_eq!(conv.to.asset.decimals, 6);
    }

    #[test]
    fn boltz_from_lightning() {
        let info = boltz_info(None, ConversionStatus::Completed);
        let payment = lightning_payment_with_info("p1", 100_000, 3, info.clone());

        let conv = build_crosschain_conversion(&info, &payment).unwrap();
        assert_eq!(conv.provider, ConversionProvider::Boltz);
        assert_eq!(conv.status, ConversionStatus::Completed);
        assert_eq!(conv.from.chain, ConversionChain::Lightning);
        assert_eq!(conv.from.asset.ticker, "BTC");
        assert_eq!(conv.from.amount, 100_000);
        assert_eq!(conv.from.fee, 1_500);
        assert_eq!(
            conv.to.chain,
            ConversionChain::External {
                name: "solana".to_string(),
                chain_id: None,
            }
        );
        assert_eq!(conv.to.asset.ticker, "USDT");
        assert_eq!(conv.to.amount, 1_450_000);
    }

    #[test]
    fn boltz_with_delivered_amount() {
        let info = boltz_info(Some(1_440_000), ConversionStatus::Completed);
        let payment = lightning_payment_with_info("p1", 100_000, 3, info.clone());

        let conv = build_crosschain_conversion(&info, &payment).unwrap();
        assert_eq!(conv.to.amount, 1_440_000);
    }

    #[test]
    fn amm_info_returns_none_for_crosschain() {
        let info = amm_info(ConversionStatus::Completed, 0);
        let payment = spark_payment("p1", PaymentType::Send, 1_000, 0, info.clone());

        assert!(build_crosschain_conversion(&info, &payment).is_none());
    }

    // --- build_conversions ordering tests ---

    fn amm_child_token(id: &str, ptype: PaymentType) -> Payment {
        token_payment(
            id,
            ptype,
            1_500_000,
            0,
            amm_info(ConversionStatus::Completed, 10),
        )
    }

    fn amm_child_spark(id: &str, ptype: PaymentType) -> Payment {
        spark_payment(
            id,
            ptype,
            1_500,
            0,
            amm_info(ConversionStatus::Completed, 0),
        )
    }

    #[test]
    fn send_amm_only() {
        let parent = parent_send_no_crosschain();
        let children = vec![
            amm_child_token("c_send", PaymentType::Send),
            amm_child_spark("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0].provider, ConversionProvider::Amm);
    }

    #[test]
    fn send_crosschain_only() {
        let parent = parent_send_lightning(orchestra_info(ConversionStatus::Pending));
        let conversions = build_conversions(&parent, None);
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0].provider, ConversionProvider::Orchestra);
    }

    #[test]
    fn send_combined_amm_then_crosschain() {
        let parent = parent_send_lightning(boltz_info(None, ConversionStatus::Completed));
        let children = vec![
            amm_child_token("c_send", PaymentType::Send),
            amm_child_spark("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 2);
        assert_eq!(conversions[0].provider, ConversionProvider::Amm);
        assert_eq!(conversions[1].provider, ConversionProvider::Boltz);
    }

    #[test]
    fn receive_amm_only() {
        let parent = parent_receive_no_crosschain();
        let children = vec![
            amm_child_spark("c_send", PaymentType::Send),
            amm_child_token("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0].provider, ConversionProvider::Amm);
    }

    #[test]
    fn receive_combined_crosschain_then_amm() {
        let mut parent = parent_receive_no_crosschain();
        parent.details = Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: Some(orchestra_info(ConversionStatus::Pending)),
        });
        let children = vec![
            amm_child_spark("c_send", PaymentType::Send),
            amm_child_token("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 2);
        assert_eq!(conversions[0].provider, ConversionProvider::Orchestra);
        assert_eq!(conversions[1].provider, ConversionProvider::Amm);
    }

    #[test]
    fn pending_no_children() {
        let mut parent = parent_send_no_crosschain();
        parent.conversion_details = Some(ConversionDetails {
            status: ConversionStatus::Pending,
            conversions: vec![],
        });

        let conversions = build_conversions(&parent, None);
        assert!(conversions.is_empty());
    }

    // --- fold_conversion_status tests (comment #3) ---

    fn conv_with_status(status: ConversionStatus) -> Conversion {
        Conversion {
            provider: ConversionProvider::Amm,
            status,
            from: ConversionSide {
                chain: ConversionChain::Spark,
                asset: ConversionAsset {
                    ticker: "BTC".into(),
                    identifier: None,
                    decimals: 0,
                },
                amount: 0,
                fee: 0,
            },
            to: ConversionSide {
                chain: ConversionChain::Spark,
                asset: ConversionAsset {
                    ticker: "USDB".into(),
                    identifier: Some("token123".into()),
                    decimals: 6,
                },
                amount: 0,
                fee: 0,
            },
            amount_adjustment: None,
        }
    }

    #[test]
    fn fold_status_completed_plus_pending_is_pending() {
        let steps = vec![
            conv_with_status(ConversionStatus::Completed),
            conv_with_status(ConversionStatus::Pending),
        ];
        assert_eq!(fold_conversion_status(&steps), ConversionStatus::Pending);
    }

    #[test]
    fn fold_status_failed_dominates() {
        let steps = vec![
            conv_with_status(ConversionStatus::Completed),
            conv_with_status(ConversionStatus::Failed),
            conv_with_status(ConversionStatus::Pending),
        ];
        assert_eq!(fold_conversion_status(&steps), ConversionStatus::Failed);
    }

    #[test]
    fn fold_status_refund_needed_dominates_pending() {
        let steps = vec![
            conv_with_status(ConversionStatus::Pending),
            conv_with_status(ConversionStatus::RefundNeeded),
        ];
        assert_eq!(
            fold_conversion_status(&steps),
            ConversionStatus::RefundNeeded
        );
    }

    #[test]
    fn fold_status_all_completed() {
        let steps = vec![
            conv_with_status(ConversionStatus::Completed),
            conv_with_status(ConversionStatus::Completed),
        ];
        assert_eq!(fold_conversion_status(&steps), ConversionStatus::Completed);
    }

    #[test]
    fn fold_status_empty_defaults_completed() {
        assert_eq!(fold_conversion_status(&[]), ConversionStatus::Completed);
    }
}
