use std::str::FromStr;

use bitcoin::hashes::sha256;
use bitcoin::secp256k1::PublicKey;
use platform_utils::time::{Duration, SystemTime};
use spark_wallet::{InvoiceDescription, LightningReceivePayment, Preimage};
use tracing::debug;

use crate::{
    ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse,
    cross_chain::{
        CrossChainReceivePrepared, CrossChainRoutePair, DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS,
        MAX_CROSS_CHAIN_SLIPPAGE_BPS, MIN_CROSS_CHAIN_SLIPPAGE_BPS, SparkAsset,
    },
    error::SdkError,
    models::{Payment, ReceivePaymentMethod, ReceivePaymentRequest, ReceivePaymentResponse},
};

use super::super::{BreezSdk, helpers::get_deposit_address};

pub(super) async fn receive_payment(
    sdk: &BreezSdk,
    request: ReceivePaymentRequest,
) -> Result<ReceivePaymentResponse, SdkError> {
    sdk.maybe_ensure_spark_private_mode_initialized().await?;
    match request.payment_method {
        ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
            fee: 0,
            cross_chain_info: None,
            payment_request: sdk
                .spark_wallet
                .get_spark_address()?
                .to_address_string()
                .map_err(|e| {
                    SdkError::Generic(format!("Failed to convert Spark address to string: {e}"))
                })?,
        }),
        ReceivePaymentMethod::SparkInvoice {
            amount,
            token_identifier,
            expiry_time,
            description,
            sender_public_key,
        } => {
            let sender_public_key = sender_public_key
                .map(|key| PublicKey::from_str(&key))
                .transpose()
                .map_err(|_| SdkError::InvalidInput("Invalid sender public key".to_string()))?;
            let invoice = sdk
                .spark_wallet
                .create_spark_invoice(
                    amount,
                    token_identifier.clone(),
                    expiry_time
                        .map(|time| {
                            SystemTime::UNIX_EPOCH
                                .checked_add(Duration::from_secs(time))
                                .ok_or(SdkError::Generic("Invalid expiry time".to_string()))
                        })
                        .transpose()?,
                    description,
                    sender_public_key,
                )
                .await?;
            Ok(ReceivePaymentResponse {
                fee: 0,
                cross_chain_info: None,
                payment_request: invoice,
            })
        }
        ReceivePaymentMethod::BitcoinAddress { new_address } => {
            let address =
                get_deposit_address(&sdk.spark_wallet, new_address.unwrap_or(false)).await?;
            Ok(ReceivePaymentResponse {
                payment_request: address,
                fee: 0,
                cross_chain_info: None,
            })
        }
        ReceivePaymentMethod::Bolt11Invoice {
            description,
            amount_sats,
            expiry_secs,
            payment_hash,
            receiver_identity_public_key,
        } => {
            receive_bolt11_invoice(
                sdk,
                description,
                amount_sats,
                expiry_secs,
                payment_hash,
                receiver_identity_public_key,
            )
            .await
        }
        ReceivePaymentMethod::CrossChain {
            route,
            amount,
            destination,
            fee_mode,
            max_slippage_bps,
            target_overpay_bps,
        } => {
            receive_cross_chain(
                sdk,
                route,
                amount,
                destination,
                fee_mode,
                max_slippage_bps,
                target_overpay_bps,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn receive_cross_chain(
    sdk: &BreezSdk,
    route: CrossChainRoutePair,
    amount: u128,
    destination: Option<SparkAsset>,
    fee_mode: Option<crate::cross_chain::CrossChainFeeMode>,
    max_slippage_bps: Option<u32>,
    target_overpay_bps: Option<u32>,
) -> Result<ReceivePaymentResponse, SdkError> {
    if amount == 0 {
        return Err(SdkError::InvalidInput(
            "Cross-chain receive amount must be greater than zero.".to_string(),
        ));
    }
    // `get_cross_chain_routes` filters the discovery list to USD-stable
    // sources. Re-check here so a hand-built route can't bypass the USD-par
    // assumption that `convert_receive_amount_to_provider_units` relies on.
    if !crate::cross_chain::is_usd_stable_asset(&route.asset) {
        return Err(SdkError::InvalidInput(format!(
            "Cross-chain receive source asset must be USD-stable. Got {}",
            route.asset
        )));
    }
    let slippage = max_slippage_bps.unwrap_or(DEFAULT_CROSS_CHAIN_SLIPPAGE_BPS);
    if !(MIN_CROSS_CHAIN_SLIPPAGE_BPS..=MAX_CROSS_CHAIN_SLIPPAGE_BPS).contains(&slippage) {
        return Err(SdkError::InvalidInput(format!(
            "Cross-chain max_slippage_bps must be in \
             {MIN_CROSS_CHAIN_SLIPPAGE_BPS} to {MAX_CROSS_CHAIN_SLIPPAGE_BPS}. \
             Got {slippage}."
        )));
    }
    let fee_mode = fee_mode.unwrap_or(crate::cross_chain::CrossChainFeeMode::FeesExcluded);
    let overpay_bps = crate::cross_chain::resolve_target_overpay_bps(
        target_overpay_bps,
        sdk.config
            .cross_chain_config
            .as_ref()
            .and_then(|c| c.default_target_overpay_bps),
    )?;

    let resolved_destination = resolve_receive_destination(sdk, &route, destination).await?;

    let provider_amount = convert_receive_amount_to_provider_units(
        sdk,
        &route,
        &resolved_destination,
        fee_mode,
        amount,
    )
    .await?;

    let service = sdk.cross_chain_context.get(route.provider)?.clone();

    let recipient = sdk
        .spark_wallet
        .get_spark_address()?
        .to_address_string()
        .map_err(|e| {
            SdkError::Generic(format!("Failed to convert Spark address to string: {e}"))
        })?;

    debug!(
        "Cross-chain receive: fee_mode={fee_mode:?} source_amount={amount} \
         source_decimals={} provider_amount={provider_amount} overpay_bps={overpay_bps} \
         slippage_bps={slippage}",
        route.decimals,
    );

    let CrossChainReceivePrepared {
        payment_request,
        info,
    } = service
        .prepare_receive(
            &route,
            &recipient,
            provider_amount,
            slippage,
            &resolved_destination,
            fee_mode,
            overpay_bps,
        )
        .await?;

    Ok(ReceivePaymentResponse {
        payment_request,
        fee: 0,
        cross_chain_info: Some(info),
    })
}

/// Converts the caller-facing receive amount (in `route.decimals` base
/// units of the source asset, at USD parity for USD-stable sources) into
/// the units the provider layer expects.
///
/// - `FeesIncluded` (any destination): identity. The source-asset amount
///   IS the sender's deposit, and the receiver gets it minus fees.
/// - `FeesExcluded` + USDB destination: rescale source-asset units to USDB
///   (6dp), at par.
/// - `FeesExcluded` + BTC destination: convert source-asset units to sats
///   via the live BTC/USD rate, at USD parity.
async fn convert_receive_amount_to_provider_units(
    sdk: &BreezSdk,
    route: &CrossChainRoutePair,
    destination: &SparkAsset,
    fee_mode: crate::cross_chain::CrossChainFeeMode,
    amount: u128,
) -> Result<u128, SdkError> {
    use crate::cross_chain::{
        CrossChainFeeMode, convert_source_amount_to_sats, fetch_btc_usd_rate, rescale_decimals,
    };
    let src_decimals = u32::from(route.decimals);
    match (fee_mode, destination) {
        (CrossChainFeeMode::FeesIncluded, _) => Ok(amount),
        (CrossChainFeeMode::FeesExcluded, SparkAsset::Token { .. }) => {
            rescale_decimals(amount, src_decimals, 6)
        }
        (CrossChainFeeMode::FeesExcluded, SparkAsset::Bitcoin) => {
            let btc_usd = fetch_btc_usd_rate(sdk.fiat_service.as_ref()).await?;
            convert_source_amount_to_sats(amount, src_decimals, btc_usd)
        }
    }
}

/// Picks a Spark-side destination asset for a cross-chain receive.
///
/// * A caller-supplied asset is honoured only if it appears in
///   `route.accepted_assets`. Otherwise `InvalidInput`.
/// * When unset, prefers the wallet's active stable-balance token if the
///   route supports it, otherwise Bitcoin. Returns `InvalidInput` if the
///   route exposes neither.
async fn resolve_receive_destination(
    sdk: &BreezSdk,
    route: &CrossChainRoutePair,
    requested: Option<SparkAsset>,
) -> Result<SparkAsset, SdkError> {
    if let Some(asset) = requested {
        if route.accepted_assets.contains(&asset) {
            return Ok(asset);
        }
        return Err(SdkError::InvalidInput(format!(
            "Requested destination {asset:?} is not supported by this route. \
             Pick one of route.accepted_assets."
        )));
    }
    if let Some(sb) = &sdk.stable_balance
        && let Some(token_identifier) = sb.get_active_token_identifier().await
    {
        let token_asset = SparkAsset::Token { token_identifier };
        if route.accepted_assets.contains(&token_asset) {
            return Ok(token_asset);
        }
    }
    if route.accepted_assets.contains(&SparkAsset::Bitcoin) {
        return Ok(SparkAsset::Bitcoin);
    }
    Err(SdkError::InvalidInput(
        "Route exposes no usable Spark destination (neither Bitcoin nor a supported token)."
            .to_string(),
    ))
}

pub(super) async fn claim_htlc_payment(
    sdk: &BreezSdk,
    request: ClaimHtlcPaymentRequest,
) -> Result<ClaimHtlcPaymentResponse, SdkError> {
    let preimage = Preimage::from_hex(&request.preimage)
        .map_err(|_| SdkError::InvalidInput("Invalid preimage".to_string()))?;
    let payment_hash = preimage.compute_hash();

    // Check if there is a claimable HTLC with the given payment hash
    let claimable_htlc_transfers = sdk.spark_wallet.list_claimable_htlc_transfers(None).await?;
    if !claimable_htlc_transfers
        .iter()
        .filter_map(|t| t.htlc_preimage_request.as_ref())
        .any(|p| p.payment_hash == payment_hash)
    {
        return Err(SdkError::InvalidInput(
            "No claimable HTLC with the given payment hash".to_string(),
        ));
    }

    let transfer = sdk.spark_wallet.claim_htlc(&preimage).await?;
    let payment: Payment = transfer.try_into()?;

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(ClaimHtlcPaymentResponse { payment })
}

pub(super) async fn receive_bolt11_invoice(
    sdk: &BreezSdk,
    description: String,
    amount_sats: Option<u64>,
    expiry_secs: Option<u32>,
    payment_hash: Option<String>,
    receiver_identity_public_key: Option<String>,
) -> Result<ReceivePaymentResponse, SdkError> {
    let receive = receive_bolt11_invoice_inner(
        sdk,
        description,
        amount_sats,
        expiry_secs,
        payment_hash,
        receiver_identity_public_key,
    )
    .await?;
    Ok(ReceivePaymentResponse {
        payment_request: receive.invoice,
        fee: 0,
        cross_chain_info: None,
    })
}

/// Internal variant of [`receive_bolt11_invoice`] that keeps the
/// full SSP receive object (id + invoice + status + …). Used by
/// `lnurl_withdraw` to get the SSP id for the synchronous wait via
/// `WaitForPaymentIdentifier::LightningReceive`.
pub(super) async fn receive_bolt11_invoice_inner(
    sdk: &BreezSdk,
    description: String,
    amount_sats: Option<u64>,
    expiry_secs: Option<u32>,
    payment_hash: Option<String>,
    receiver_identity_public_key: Option<String>,
) -> Result<LightningReceivePayment, SdkError> {
    let receiver_identity_public_key = receiver_identity_public_key
        .map(|key| PublicKey::from_str(&key))
        .transpose()
        .map_err(|_| SdkError::InvalidInput("Invalid receiver identity public key".to_string()))?;
    let receive = if let Some(payment_hash_hex) = payment_hash {
        let hash = sha256::Hash::from_str(&payment_hash_hex)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid payment hash: {e}")))?;
        sdk.spark_wallet
            .create_hodl_lightning_invoice(
                amount_sats.unwrap_or_default(),
                Some(InvoiceDescription::Memo(description.clone())),
                hash,
                receiver_identity_public_key,
                expiry_secs,
            )
            .await?
    } else {
        sdk.spark_wallet
            .create_lightning_invoice(
                amount_sats.unwrap_or_default(),
                Some(InvoiceDescription::Memo(description.clone())),
                receiver_identity_public_key,
                expiry_secs,
                sdk.config.prefer_spark_over_lightning,
            )
            .await?
    };
    Ok(receive)
}
