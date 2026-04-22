use std::str::FromStr;
use std::sync::Arc;

use spark_wallet::SparkWallet;
use tracing::{debug, info, warn};

use crate::{
    ConversionInfo, ConversionStatus, EventEmitter, Payment, PaymentDetails, PaymentMetadata,
    PaymentType, Storage,
    error::SdkError,
    events::SdkEvent,
    models::{build_amm_conversion, build_crosschain_conversion},
    persist::ObjectCacheRepository,
    utils::token::token_transaction_to_payments,
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

/// Gets the payment from storage to include already stored metadata and conversion details.
/// Emits the appropriate event based on its status. Falls back to the provided
/// payment if the storage lookup fails.
pub(crate) async fn get_payment_and_emit_event(
    storage: &Arc<dyn Storage>,
    event_emitter: &EventEmitter,
    payment: Payment,
) {
    let payment =
        match get_payment_with_conversion_details(payment.id.clone(), Arc::clone(storage)).await {
            Ok(payment) => payment,
            Err(e) => {
                warn!("Failed to fetch payment from storage: {e:?}");
                payment
            }
        };
    info!("Emitting payment event: {payment:?}");
    event_emitter.emit(&SdkEvent::from_payment(payment)).await;
}

/// Gets a payment from storage by ID to include already stored payment metadata
/// and then enriches it with conversions by looking up related child payments
/// and the payment's own conversion info.
///
/// Builds conversions when either:
/// - `conversion_details` is already set (AMM conversions via stable balance), or
/// - the payment carries cross-chain `ConversionInfo` (Orchestra/Boltz sends)
pub async fn get_payment_with_conversion_details(
    id: String,
    storage: Arc<dyn Storage>,
) -> Result<Payment, SdkError> {
    let mut payment = storage.get_payment_by_id(id).await?;
    enrich_payment_conversions(&mut payment, &storage).await?;
    Ok(payment)
}

/// Enriches a single payment with its conversion details if applicable.
async fn enrich_payment_conversions(
    payment: &mut Payment,
    storage: &Arc<dyn Storage>,
) -> Result<(), SdkError> {
    let has_conversion_details = payment.conversion_details.is_some();
    let has_crosschain_info = extract_conversion_info(payment.details.clone())
        .is_some_and(|info| !matches!(info, ConversionInfo::Amm { .. }));

    if !has_conversion_details && !has_crosschain_info {
        return Ok(());
    }

    // Fetch child payments if conversion_details is set (AMM case)
    let child_payments = if has_conversion_details {
        let map = storage
            .get_payments_by_parent_ids(vec![payment.id.clone()])
            .await?;
        map.get(&payment.id).cloned()
    } else {
        None
    };

    let conversions = build_conversions(payment, child_payments.as_deref());

    if !conversions.is_empty() {
        if let Some(ref mut cd) = payment.conversion_details {
            cd.conversions = conversions;
        } else {
            // Cross-chain send without pre-set conversion_details — derive status from info
            let status = extract_conversion_info(payment.details.clone())
                .map_or(ConversionStatus::Completed, |info| info.status().clone());
            payment.conversion_details = Some(crate::models::ConversionDetails {
                status,
                conversions,
            });
        }
    }

    Ok(())
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
) -> Vec<crate::models::Conversion> {
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
            // Only send child exists: parent is the receive side (auto-conversion, self-transfer)
            (Some(s), None) => Some((s, payment)),
            // Only receive child exists: parent is the send side
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

/// Resolves a Spark transfer ID or token transaction hash to a payment ID.
///
/// If `identifier` is a valid [`TransferId`] it is returned directly (Spark
/// transfers use the transfer ID as the payment ID). Otherwise we look up
/// the token transaction by hash and extract the payment ID from it.
///
/// Used by both `FlashnetTokenConverter` (AMM conversions) and
/// `OrchestraService` (cross-chain sends) when attaching metadata to the
/// outgoing Spark payment.
pub(crate) async fn resolve_payment_id(
    identifier: &str,
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    tx_inputs_are_ours: bool,
) -> Result<String, SdkError> {
    use spark_wallet::TransferId;

    debug!("Resolving payment id for identifier: {identifier}");

    if let Ok(transfer_id) = TransferId::from_str(identifier) {
        return Ok(transfer_id.to_string());
    }

    // It's a token transaction hash — look up the transaction and derive the payment.
    let token_transactions = spark_wallet
        .get_token_transactions_by_hashes(vec![identifier.to_string()])
        .await
        .map_err(|e| SdkError::Generic(format!("Failed to get token transactions: {e}")))?;

    let token_transaction = token_transactions
        .first()
        .ok_or_else(|| SdkError::Generic("Token transaction not found".to_string()))?;

    let object_repository = ObjectCacheRepository::new(Arc::clone(storage));
    let payments = token_transaction_to_payments(
        spark_wallet,
        &object_repository,
        token_transaction,
        tx_inputs_are_ours,
    )
    .await?;

    payments
        .first()
        .map(|p| p.id.clone())
        .ok_or_else(|| SdkError::Generic("Payment id not found for token transaction".to_string()))
}

/// Inserts payment metadata by first resolving the identifier to a payment ID.
/// If the payment ID can't be resolved yet (async sync hasn't processed the
/// transfer), caches the metadata for later attachment.
///
/// Returns the resolved payment ID, or the raw identifier if it was cached.
pub(crate) async fn insert_or_cache_payment_metadata(
    identifier: &str,
    metadata: PaymentMetadata,
    spark_wallet: &SparkWallet,
    storage: &Arc<dyn Storage>,
    tx_inputs_are_ours: bool,
) -> Result<String, SdkError> {
    match resolve_payment_id(identifier, spark_wallet, storage, tx_inputs_are_ours).await {
        Ok(payment_id) => {
            debug!("Resolved payment id {payment_id} for identifier {identifier}");
            storage
                .insert_payment_metadata(payment_id.clone(), metadata)
                .await
                .map_err(|e| {
                    SdkError::Generic(format!("Failed to insert payment metadata: {e}"))
                })?;
            Ok(payment_id)
        }
        Err(e) => {
            debug!("Could not resolve payment id for {identifier}: {e}, caching metadata");
            let cache = ObjectCacheRepository::new(Arc::clone(storage));
            cache
                .save_payment_metadata(identifier, &metadata)
                .await
                .map_err(|e| SdkError::Generic(format!("Failed to cache payment metadata: {e}")))?;
            Ok(identifier.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ConversionInfo, ConversionStatus, SparkHtlcDetails, SparkHtlcStatus,
        models::{
            ConversionDetails, ConversionProvider, Payment, PaymentDetails, PaymentMethod,
            PaymentStatus, PaymentType, TokenMetadata, TokenTransactionType,
        },
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

    fn amm_info() -> ConversionInfo {
        ConversionInfo::Amm {
            pool_id: "pool_1".to_string(),
            conversion_id: "conv_1".to_string(),
            status: ConversionStatus::Completed,
            fee: Some(10),
            purpose: None,
            amount_adjustment: None,
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

    fn token_child(id: &str, ptype: PaymentType) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: ptype,
            status: PaymentStatus::Completed,
            amount: 1_500_000,
            fees: 0,
            timestamp: 1000,
            method: PaymentMethod::Token,
            details: Some(PaymentDetails::Token {
                metadata: test_token_metadata(),
                tx_hash: "tx_1".to_string(),
                tx_type: TokenTransactionType::Transfer,
                invoice_details: None,
                conversion_info: Some(amm_info()),
            }),
            conversion_details: None,
        }
    }

    fn spark_child(id: &str, ptype: PaymentType) -> Payment {
        Payment {
            id: id.to_string(),
            payment_type: ptype,
            status: PaymentStatus::Completed,
            amount: 1_500,
            fees: 0,
            timestamp: 1000,
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: Some(amm_info()),
            }),
            conversion_details: None,
        }
    }

    fn boltz_info() -> ConversionInfo {
        ConversionInfo::Boltz {
            swap_id: "swap_1".to_string(),
            chain: "solana".to_string(),
            asset: "USDT".to_string(),
            recipient_address: "So1ana".to_string(),
            invoice: "lnbc1000n1p".to_string(),
            invoice_amount_sats: 100_000,
            estimated_out: 1_450_000,
            delivered_amount: None,
            lz_guid: None,
            status: ConversionStatus::Completed,
            fee: Some(1_500),
            max_slippage_bps: 100,
            quote_degraded: false,
            asset_decimals: Some(6),
        }
    }

    fn orchestra_info() -> ConversionInfo {
        ConversionInfo::Orchestra {
            order_id: "ord_1".to_string(),
            quote_id: "q_1".to_string(),
            chain: "base".to_string(),
            asset: "USDC".to_string(),
            recipient_address: "0x1234".to_string(),
            estimated_out: 99_500_000,
            delivered_amount: None,
            status: ConversionStatus::Pending,
            fee: Some(500),
            read_token: None,
            asset_decimals: Some(6),
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

    // --- build_conversions ordering tests ---

    #[test]
    fn send_amm_only() {
        let parent = parent_send_no_crosschain();
        let children = vec![
            token_child("c_send", PaymentType::Send),
            spark_child("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0].provider, ConversionProvider::Amm);
    }

    #[test]
    fn send_crosschain_only() {
        let parent = parent_send_lightning(orchestra_info());
        let conversions = build_conversions(&parent, None);
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0].provider, ConversionProvider::Orchestra);
    }

    #[test]
    fn send_combined_amm_then_crosschain() {
        let parent = parent_send_lightning(boltz_info());
        let children = vec![
            token_child("c_send", PaymentType::Send),
            spark_child("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 2);
        assert_eq!(
            conversions[0].provider,
            ConversionProvider::Amm,
            "AMM should be first for sends"
        );
        assert_eq!(
            conversions[1].provider,
            ConversionProvider::Boltz,
            "Cross-chain should be second for sends"
        );
    }

    #[test]
    fn receive_amm_only() {
        let parent = parent_receive_no_crosschain();
        let children = vec![
            spark_child("c_send", PaymentType::Send),
            token_child("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 1);
        assert_eq!(conversions[0].provider, ConversionProvider::Amm);
    }

    #[test]
    fn receive_combined_crosschain_then_amm() {
        let mut parent = parent_receive_no_crosschain();
        // Add orchestra info to the receive parent
        parent.details = Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: Some(orchestra_info()),
        });
        let children = vec![
            spark_child("c_send", PaymentType::Send),
            token_child("c_recv", PaymentType::Receive),
        ];

        let conversions = build_conversions(&parent, Some(&children));
        assert_eq!(conversions.len(), 2);
        assert_eq!(
            conversions[0].provider,
            ConversionProvider::Orchestra,
            "Cross-chain should be first for receives"
        );
        assert_eq!(
            conversions[1].provider,
            ConversionProvider::Amm,
            "AMM should be second for receives"
        );
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
}
