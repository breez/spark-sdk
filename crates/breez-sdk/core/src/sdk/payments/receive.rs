use std::str::FromStr;

use bitcoin::hashes::sha256;
use bitcoin::secp256k1::PublicKey;
use platform_utils::time::{Duration, SystemTime};
use spark_wallet::{InvoiceDescription, InvoicePreimage, LightningReceivePayment, Preimage};
use tracing::debug;

use breez_sdk_common::fiat::FiatService;

use crate::{
    ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse, CrossChainConfig,
    cross_chain::{
        CrossChainContext, CrossChainFeeMode, CrossChainReceivePrepared, CrossChainRoutePair,
        DeliveryMethod, SparkAsset,
    },
    error::SdkError,
    models::{Payment, ReceivePaymentMethod, ReceivePaymentRequest, ReceivePaymentResponse},
};

use super::super::{BreezSdk, helpers::get_deposit_address};
use super::validation::resolve_slippage_bps;

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
            let address = get_deposit_address(
                &sdk.spark_wallet,
                &sdk.storage,
                new_address.unwrap_or(false),
            )
            .await?;
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
            preimage,
        } => {
            receive_bolt11_invoice(
                sdk,
                description,
                amount_sats,
                expiry_secs,
                payment_hash,
                receiver_identity_public_key,
                preimage,
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
        } => receive_cross_chain(
            sdk,
            route,
            amount,
            destination,
            fee_mode,
            max_slippage_bps,
            target_overpay_bps,
        )
        .await
        .map(Into::into),
    }
}

impl From<CrossChainReceivePrepared> for ReceivePaymentResponse {
    fn from(prepared: CrossChainReceivePrepared) -> Self {
        ReceivePaymentResponse {
            payment_request: prepared.payment_request,
            fee: 0,
            cross_chain_info: Some(prepared.info),
        }
    }
}

/// A cross-chain receive into this wallet, tracked as a payment.
/// `destination` picks the Spark-side asset, defaulting as
/// [`resolve_receive_destination`] describes.
#[allow(clippy::too_many_arguments)]
async fn receive_cross_chain(
    sdk: &BreezSdk,
    route: CrossChainRoutePair,
    amount: u128,
    destination: Option<SparkAsset>,
    fee_mode: Option<CrossChainFeeMode>,
    max_slippage_bps: Option<u32>,
    target_overpay_bps: Option<u32>,
) -> Result<CrossChainReceivePrepared, SdkError> {
    let params = resolve_cross_chain_receive_params(
        &sdk.cross_chain_context,
        sdk.config.cross_chain_config.as_ref(),
        &route,
        amount,
        DeliveryMethod::Spark,
        fee_mode,
        max_slippage_bps,
        target_overpay_bps,
    )?;
    let destination = resolve_receive_destination(sdk, &route, destination).await?;
    let address = sdk
        .spark_wallet
        .get_spark_address()?
        .to_address_string()
        .map_err(|e| {
            SdkError::Generic(format!("Failed to convert Spark address to string: {e}"))
        })?;
    quote_cross_chain_receive(
        &sdk.cross_chain_context,
        &route,
        amount,
        &destination,
        &params,
        &address,
        None,
    )
    .await
}

/// Settings a cross-chain receive quote is made with.
pub(in crate::sdk) struct CrossChainReceiveParams {
    delivery_method: DeliveryMethod,
    fee_mode: CrossChainFeeMode,
    slippage_bps: u32,
    overpay_bps: u32,
}

/// Checks a cross-chain receive request, and resolves its fee mode, slippage
/// and overpay against `cross_chain_config` and the built-in defaults.
#[allow(clippy::too_many_arguments)]
pub(in crate::sdk) fn resolve_cross_chain_receive_params(
    cross_chain_context: &CrossChainContext,
    cross_chain_config: Option<&CrossChainConfig>,
    route: &CrossChainRoutePair,
    amount: u128,
    delivery_method: DeliveryMethod,
    fee_mode: Option<CrossChainFeeMode>,
    max_slippage_bps: Option<u32>,
    target_overpay_bps: Option<u32>,
) -> Result<CrossChainReceiveParams, SdkError> {
    cross_chain_context.ensure_enabled()?;
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
    let slippage_bps = resolve_slippage_bps(
        max_slippage_bps,
        cross_chain_config.and_then(|c| c.default_slippage_bps),
    )?;
    let fee_mode = fee_mode.unwrap_or(CrossChainFeeMode::FeesExcluded);
    let overpay_bps = crate::cross_chain::resolve_target_overpay_bps(
        target_overpay_bps,
        cross_chain_config.and_then(|c| c.default_target_overpay_bps),
    )?;
    crate::cross_chain::ensure_route_delivers_over(route, delivery_method)?;
    Ok(CrossChainReceiveParams {
        delivery_method,
        fee_mode,
        slippage_bps,
        overpay_bps,
    })
}

/// Quotes a cross-chain receive that delivers `destination` to `recipient`.
/// `refund_address` is where the provider returns the deposit if delivery
/// fails.
pub(in crate::sdk) async fn quote_cross_chain_receive(
    cross_chain_context: &CrossChainContext,
    route: &CrossChainRoutePair,
    amount: u128,
    destination: &SparkAsset,
    params: &CrossChainReceiveParams,
    recipient: &str,
    refund_address: Option<&str>,
) -> Result<CrossChainReceivePrepared, SdkError> {
    let provider_amount = convert_receive_amount_to_provider_units(
        cross_chain_context.fiat_service().as_ref(),
        route,
        destination,
        params.fee_mode,
        amount,
    )
    .await?;
    if provider_amount == 0 {
        return Err(SdkError::InvalidInput(format!(
            "Cross-chain receive amount {amount} is too small for this route: it converts to \
             zero units of the requested destination."
        )));
    }

    let service = cross_chain_context.get(route.provider)?.clone();

    debug!(
        "Cross-chain receive: fee_mode={:?} source_amount={amount} \
         source_decimals={} provider_amount={provider_amount} overpay_bps={} \
         slippage_bps={}",
        params.fee_mode, route.decimals, params.overpay_bps, params.slippage_bps,
    );

    service
        .prepare_receive(
            route,
            recipient,
            provider_amount,
            params.slippage_bps,
            destination,
            params.fee_mode,
            params.overpay_bps,
            params.delivery_method,
            refund_address,
        )
        .await
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
    fiat_service: &dyn FiatService,
    route: &CrossChainRoutePair,
    destination: &SparkAsset,
    fee_mode: crate::cross_chain::CrossChainFeeMode,
    amount: u128,
) -> Result<u128, SdkError> {
    use crate::cross_chain::{convert_source_amount_to_sats, fetch_btc_usd_rate, rescale_decimals};
    let src_decimals = u32::from(route.decimals);
    match (fee_mode, destination) {
        (CrossChainFeeMode::FeesIncluded, _) => Ok(amount),
        (CrossChainFeeMode::FeesExcluded, SparkAsset::Token { .. }) => {
            rescale_decimals(amount, src_decimals, 6)
        }
        (CrossChainFeeMode::FeesExcluded, SparkAsset::Bitcoin) => {
            let btc_usd = fetch_btc_usd_rate(fiat_service).await?;
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
        return validate_receive_destination(route, asset);
    }
    if let Some(sb) = &sdk.stable_balance
        && let Some(token_identifier) = sb.get_active_token_identifier().await
    {
        let token_asset = SparkAsset::Token { token_identifier };
        if route.accepts_asset(&token_asset) {
            return Ok(token_asset);
        }
    }
    if route.accepts_asset(&SparkAsset::Bitcoin) {
        return Ok(SparkAsset::Bitcoin);
    }
    Err(SdkError::InvalidInput(
        "Route exposes no usable Spark destination (neither Bitcoin nor a supported token)."
            .to_string(),
    ))
}

/// Accepts `asset` as a receive destination only if `route` lists it in
/// `accepted_assets`.
pub(in crate::sdk) fn validate_receive_destination(
    route: &CrossChainRoutePair,
    asset: SparkAsset,
) -> Result<SparkAsset, SdkError> {
    if route.accepts_asset(&asset) {
        return Ok(asset);
    }
    Err(SdkError::InvalidInput(format!(
        "Requested destination {asset:?} is not supported by this route. \
         Pick one of route.accepted_assets."
    )))
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
    preimage: Option<String>,
) -> Result<ReceivePaymentResponse, SdkError> {
    let receive = receive_bolt11_invoice_inner(
        sdk,
        description,
        amount_sats,
        expiry_secs,
        payment_hash,
        receiver_identity_public_key,
        preimage,
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
    preimage: Option<String>,
) -> Result<LightningReceivePayment, SdkError> {
    let receiver_identity_public_key = receiver_identity_public_key
        .map(|key| PublicKey::from_str(&key))
        .transpose()
        .map_err(|_| SdkError::InvalidInput("Invalid receiver identity public key".to_string()))?;
    if payment_hash.is_some() && preimage.is_some() {
        return Err(SdkError::InvalidInput(
            "Set either a payment hash or a preimage, not both".to_string(),
        ));
    }
    let (invoice_preimage, include_spark_address) = if let Some(payment_hash_hex) = payment_hash {
        let hash = sha256::Hash::from_str(&payment_hash_hex)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid payment hash: {e}")))?;
        // No Spark address: a payer who pays by a direct Spark transfer bypasses the hold.
        (InvoicePreimage::Hodl(hash), false)
    } else if let Some(preimage_hex) = preimage {
        let preimage = Preimage::from_hex(&preimage_hex).map_err(|_| {
            SdkError::InvalidInput("Invalid preimage: expected 32 hex-encoded bytes".to_string())
        })?;
        // No Spark address: a payer who pays by a direct Spark transfer never learns the preimage.
        (InvoicePreimage::FromCaller(preimage), false)
    } else {
        (
            InvoicePreimage::FromSigner,
            sdk.config.prefer_spark_over_lightning,
        )
    };
    Ok(sdk
        .spark_wallet
        .create_lightning_invoice(
            amount_sats.unwrap_or_default(),
            Some(InvoiceDescription::Memo(description)),
            invoice_preimage,
            receiver_identity_public_key,
            expiry_secs,
            include_spark_address,
        )
        .await?)
}
