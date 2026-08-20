//! Builders for the per-step [`Conversion`] entries that populate the public
//! API, from AMM child payments and from cross-chain `ConversionInfo`.
//!
//! The public types live in [`crate::models`]. Assembling these entries into a
//! [`Payment`]'s conversion details is [`crate::utils::payments`]'s job.

use std::sync::Arc;

use flashnet::AssetTransfer;
use spark_wallet::SparkWallet;

use crate::{
    ConversionInfo, ConversionStatus, Payment, PaymentDetails, PaymentMetadata, Storage,
    error::SdkError,
    models::{Conversion, ConversionAsset, ConversionChain, ConversionProvider, ConversionSide},
    persist::ObjectCacheRepository,
    utils::token::token_transaction_to_payments,
};

/// Converts a freshly-produced [`AssetTransfer`] into the [`Payment`] row the
/// SDK would surface once operator-side sync catches up.
///
/// - `AssetTransfer::Spark` → uses the [`Payment::try_from`] adapter on
///   the wallet transfer (no network IO).
/// - `AssetTransfer::Token` → calls [`token_transaction_to_payments`] (cache-
///   backed metadata lookup) and filters to the output matching
///   `payment_id`. Returns `Ok(None)` when no output matches (e.g. token
///   transactions with only a change output, or a `payment_id` mismatch).
pub(crate) async fn payment_from_asset_transfer(
    transfer: AssetTransfer,
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    payment_id: &str,
) -> Result<Option<Payment>, SdkError> {
    match transfer {
        AssetTransfer::Spark(wallet_transfer) => Ok(Some(Payment::try_from(wallet_transfer)?)),
        AssetTransfer::Token(token_tx) => {
            let object_repository = ObjectCacheRepository::new(Arc::clone(storage));
            let payments =
                token_transaction_to_payments(spark_wallet, &object_repository, &token_tx, true)
                    .await?;
            Ok(payments.into_iter().find(|p| p.id == payment_id))
        }
    }
}

/// Persists `metadata` against the payment row corresponding to a
/// freshly-produced [`AssetTransfer`], resolving the payment id directly
/// from the in-hand transfer instead of round-tripping back to the
/// operators.
///
/// The string-identifier variant
/// [`crate::utils::payments::resolve_and_insert_payment_metadata`] calls
/// `get_token_transactions_by_hashes` on the token branch to re-fetch the
/// transaction it needs to derive the payment id; for callers that
/// already received the [`AssetTransfer`] from `transfer_to_deposit` /
/// `transfer_asset`, that fetch is wasted work. This helper takes the
/// transfer by reference and skips the network step.
///
/// The payment id is locally resolvable from the in-hand transfer, so the row
/// insert is attempted directly. On a storage write failure the `metadata` is
/// instead cached under `transfer.id()` (the sync-time identifier: the Spark
/// `payment.id` or the token `tx_hash`), so the next sync's
/// [`crate::sync::SparkSyncService::apply_payment_metadata`] reapplies it into
/// the row. Errors only if both the insert and the cache write fail.
pub(crate) async fn resolve_and_insert_payment_metadata_for_transfer(
    transfer: &AssetTransfer,
    metadata: PaymentMetadata,
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    tx_inputs_are_ours: bool,
) -> Result<String, SdkError> {
    let payment_id = match transfer {
        AssetTransfer::Spark(wallet_transfer) => wallet_transfer.id.to_string(),
        AssetTransfer::Token(token_tx) => {
            let object_repository = ObjectCacheRepository::new(Arc::clone(storage));
            let payments = token_transaction_to_payments(
                spark_wallet,
                &object_repository,
                token_tx,
                tx_inputs_are_ours,
            )
            .await?;
            payments.first().map(|p| p.id.clone()).ok_or_else(|| {
                SdkError::Generic(
                    "Token transaction has no outputs that produce a Payment row".to_string(),
                )
            })?
        }
    };

    // Cache key is the sync-time identifier (`transfer.id()`), not the resolved
    // `payment_id`: token payment ids carry a `:vout` suffix that
    // `apply_payment_metadata` does not key on.
    crate::utils::payments::insert_payment_metadata_with_cache_fallback(
        storage,
        payment_id.clone(),
        &transfer.id(),
        metadata,
    )
    .await?;

    Ok(payment_id)
}

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
            fee_amount,
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
                fee: fee_amount.unwrap_or(0),
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
            fee_amount,
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
                fee: fee_amount.unwrap_or(0),
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
        AmountAdjustmentReason, PaymentType, SparkHtlcDetails, SparkHtlcStatus,
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
            degradation: None,
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
            degradation: None,
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
            asset_amount_in: Some(100_000_000),
            estimated_out: 99_500_000,
            delivered_amount: None,
            status,
            fee_amount: Some(500_000),
            service_fee_amount: Some(500),
            service_fee_asset: Some("USDC".to_string()),
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
            asset_amount_in: Some(1_500_000),
            estimated_out: 1_450_000,
            delivered_amount: delivered,
            bridge_ref: None,
            status,
            fee_amount: Some(50_000),
            service_fee_amount: Some(1_500),
            service_fee_asset: None,
            max_slippage_bps: 100,
            quote_degraded: false,
            asset_decimals: 6,
            asset_contract: Some("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string()),
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
        // `to.fee` reflects the full user-visible fee in dest units
        // (= `ConversionInfo::Orchestra.fee_amount`). The provider-only
        // spread is exposed separately as `service_fee_amount`.
        assert_eq!(conv.to.fee, 500_000);
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
        assert_eq!(conv.from.fee, 0);
        assert_eq!(
            conv.to.chain,
            ConversionChain::External {
                name: "solana".to_string(),
                chain_id: None,
            }
        );
        assert_eq!(conv.to.asset.ticker, "USDT");
        assert_eq!(conv.to.amount, 1_450_000);
        assert_eq!(conv.to.fee, 50_000);
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
}
