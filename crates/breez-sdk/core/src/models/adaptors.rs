use breez_sdk_common::input::{
    self, InputType, PaymentRequestSource, SparkInvoiceDetails, parse_spark_address,
};
use spark_wallet::{
    CoopExitAcceptance, CoopExitFeeQuote, CoopExitSpeedFeeQuote, ExitSpeed, LightningSendPayment,
    LightningSendStatus, Network as SparkNetwork, PreimageRequest, PreimageRequestStatus,
    SspUserRequest, TokenTransactionStatus, TransferDirection, TransferStatus, TransferType,
    WalletTransfer,
};
use std::time::Duration;

use platform_utils::time::UNIX_EPOCH;
use tracing::{debug, warn};

use crate::{
    AutoOptimizationEvent, Fee, Network, OnchainConfirmationSpeed, OptimizationOutcome, Payment,
    PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError, SendOnchainFeeQuote,
    SendOnchainSpeedFeeQuote, SparkHtlcDetails, SparkHtlcStatus, SparkInvoicePaymentDetails,
    TokenBalance, TokenMetadata,
};

/// Feb 1, 2026 00:00:00 UTC — transfers before this may lack HTLC data on the operator.
#[allow(clippy::duration_suboptimal_units)]
const HTLC_DATA_REQUIRED_SINCE: Duration = Duration::from_secs(1_769_904_000);

/// Derive HTLC details from SSP request fields when the operator lacks the
/// `PreimageRequest`. Only allowed for old transfers (before [`HTLC_DATA_REQUIRED_SINCE`])
/// and for a share of a fee-bearing payment, whose `PreimageRequest` belongs to
/// whoever attested its quote: on a receive created on the payee's behalf, not
/// the payee. Other new transfers without HTLC data are considered an error.
fn derive_htlc_details_from_ssp(
    transfer: &WalletTransfer,
    payment_hash: &str,
    preimage: Option<&str>,
) -> Result<SparkHtlcDetails, SdkError> {
    let cutoff = UNIX_EPOCH
        .checked_add(HTLC_DATA_REQUIRED_SINCE)
        .ok_or_else(|| SdkError::Generic("HTLC cutoff time overflow".to_string()))?;
    let is_old = transfer.created_at.is_none_or(|t| t < cutoff);
    if !is_old && transfer.own_share.is_none() {
        return Err(SdkError::Generic(format!(
            "Missing HTLC details for Lightning payment transfer {}",
            transfer.id
        )));
    }

    if is_old {
        warn!(
            "Missing HTLC preimage request for Lightning transfer {}, deriving from SSP data",
            transfer.id
        );
    } else {
        debug!(
            "Deriving HTLC details for fee-bearing transfer {} from SSP data",
            transfer.id
        );
    }

    let status = match transfer.status {
        TransferStatus::Completed => SparkHtlcStatus::PreimageShared,
        TransferStatus::Expired | TransferStatus::Returned => SparkHtlcStatus::Returned,
        _ => SparkHtlcStatus::WaitingForPreimage,
    };
    let mut details = SparkHtlcDetails {
        payment_hash: payment_hash.to_string(),
        preimage: None,
        expiry_time: transfer
            .expiry_time
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs()),
        status,
    };
    reconcile_htlc_preimage(&mut details, preimage);
    Ok(details)
}

/// If the HTLC details are missing a preimage, fill it in from the given fallback and update
/// the status to [`SparkHtlcStatus::PreimageShared`] accordingly.
fn reconcile_htlc_preimage(details: &mut SparkHtlcDetails, preimage: Option<&str>) {
    if details.preimage.is_none() {
        details.preimage = preimage.map(ToString::to_string);
    }
    if details.preimage.is_some() {
        details.status = SparkHtlcStatus::PreimageShared;
    }
}

impl PaymentMethod {
    fn from_transfer(transfer: &WalletTransfer) -> Self {
        match transfer.transfer_type {
            TransferType::PreimageSwap => {
                if transfer.is_ssp_transfer {
                    PaymentMethod::Lightning
                } else {
                    PaymentMethod::Spark
                }
            }
            TransferType::CooperativeExit => PaymentMethod::Withdraw,
            TransferType::UtxoSwap => PaymentMethod::Deposit,
            TransferType::Transfer => PaymentMethod::Spark,
            _ => PaymentMethod::Unknown,
        }
    }
}

impl PaymentDetails {
    #[allow(clippy::too_many_lines)]
    fn from_transfer(transfer: &WalletTransfer) -> Result<Option<Self>, SdkError> {
        if !transfer.is_ssp_transfer {
            // Check for Spark invoice payments
            if let Some(spark_invoice) = &transfer.spark_invoice {
                let Some(InputType::SparkInvoice(invoice_details)) =
                    parse_spark_address(spark_invoice, &PaymentRequestSource::default())
                else {
                    return Err(SdkError::Generic("Invalid spark invoice".to_string()));
                };

                return Ok(Some(PaymentDetails::Spark {
                    invoice_details: Some(invoice_details.into()),
                    htlc_details: None,
                    conversion_info: None,
                }));
            }

            // Check for Spark HTLC payments (when no user request is present)
            if let Some(htlc_preimage_request) = &transfer.htlc_preimage_request {
                return Ok(Some(PaymentDetails::Spark {
                    invoice_details: None,
                    htlc_details: Some(htlc_preimage_request.clone().try_into()?),
                    conversion_info: None,
                }));
            }

            return Ok(Some(PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: None,
            }));
        }

        let Some(user_request) = &transfer.user_request else {
            return Ok(None);
        };

        let details = match user_request {
            SspUserRequest::LightningReceiveRequest(request) => {
                let invoice_details = input::parse_invoice(&request.invoice.encoded_invoice)
                    .ok_or(SdkError::Generic(
                        "Invalid invoice in SspUserRequest::LightningReceiveRequest".to_string(),
                    ))?;
                let htlc_details = if let Some(req) = &transfer.htlc_preimage_request {
                    let mut details: SparkHtlcDetails = req.clone().try_into()?;
                    reconcile_htlc_preimage(
                        &mut details,
                        request.lightning_receive_payment_preimage.as_deref(),
                    );
                    details
                } else {
                    derive_htlc_details_from_ssp(
                        transfer,
                        &request.invoice.payment_hash,
                        request.lightning_receive_payment_preimage.as_deref(),
                    )?
                };
                PaymentDetails::Lightning {
                    description: invoice_details.description,
                    invoice: request.invoice.encoded_invoice.clone(),
                    destination_pubkey: invoice_details.payee_pubkey,
                    htlc_details,
                    lnurl_pay_info: None,
                    lnurl_withdraw_info: None,
                    lnurl_receive_metadata: None,
                    conversion_info: None,
                }
            }
            SspUserRequest::LightningSendRequest(request) => {
                let invoice_details =
                    input::parse_invoice(&request.encoded_invoice).ok_or(SdkError::Generic(
                        "Invalid invoice in SspUserRequest::LightningSendRequest".to_string(),
                    ))?;
                let htlc_details = if let Some(req) = &transfer.htlc_preimage_request {
                    let mut details: SparkHtlcDetails = req.clone().try_into()?;
                    reconcile_htlc_preimage(
                        &mut details,
                        request.lightning_send_payment_preimage.as_deref(),
                    );
                    details
                } else {
                    derive_htlc_details_from_ssp(
                        transfer,
                        &invoice_details.payment_hash,
                        request.lightning_send_payment_preimage.as_deref(),
                    )?
                };
                PaymentDetails::Lightning {
                    description: invoice_details.description,
                    invoice: request.encoded_invoice.clone(),
                    destination_pubkey: invoice_details.payee_pubkey,
                    htlc_details,
                    lnurl_pay_info: None,
                    lnurl_withdraw_info: None,
                    lnurl_receive_metadata: None,
                    conversion_info: None,
                }
            }
            SspUserRequest::CoopExitRequest(request) => PaymentDetails::Withdraw {
                tx_id: request.coop_exit_txid.clone(),
            },
            SspUserRequest::LeavesSwapRequest(_) => PaymentDetails::Spark {
                invoice_details: None,
                htlc_details: None,
                conversion_info: None,
            },
            SspUserRequest::ClaimStaticDeposit(request) => {
                let Ok(vout) = u32::try_from(request.output_index) else {
                    // vout=0 is a valid output index, so we can't safely default.
                    // Returning Ok(None) mirrors the early return for a missing
                    // user_request above: the downstream branch records the deposit
                    // as Pending without details and the sync loop keeps advancing
                    // the offset. Erroring here would deadlock the whole sync until
                    // the offending transfer leaves the SSP's response.
                    warn!(
                        "Invalid output_index {} in SspUserRequest::ClaimStaticDeposit; skipping details",
                        request.output_index
                    );
                    return Ok(None);
                };
                PaymentDetails::Deposit {
                    tx_id: request.transaction_id.clone(),
                    vout,
                }
            }
        };

        Ok(Some(details))
    }
}

impl From<SparkInvoiceDetails> for SparkInvoicePaymentDetails {
    fn from(value: SparkInvoiceDetails) -> Self {
        Self {
            description: value.description,
            invoice: value.invoice,
        }
    }
}

/// A Lightning send's fee and amount. The amount is what the invoice asks for
/// when it names one: a fee-bearing send funds the SSP and the partner fee
/// payees in one transfer, so the SSP's own fee does not cover everything sent
/// beyond the amount. An amountless invoice pays what the SSP received
/// (`ssp_received_sat`) less its fee.
fn lightning_send_fee_and_amount(
    total_sat: u64,
    ssp_received_sat: u64,
    ssp_fee_sat: u64,
    invoice: &str,
) -> (u64, u64) {
    let invoice_amount_sat = input::parse_invoice(invoice)
        .and_then(|details| details.amount_msat)
        .map(|msat| msat.div_ceil(1000))
        .filter(|amount| *amount > 0 && *amount <= total_sat);
    let amount = invoice_amount_sat
        .unwrap_or_else(|| ssp_received_sat.min(total_sat).saturating_sub(ssp_fee_sat));
    (total_sat.saturating_sub(amount), amount)
}

/// A Lightning receive's fee and amount. The transfer's total covers every
/// receiver of a fee-bearing receive, while its leaves (`own_sat`) are this
/// wallet's own: what they hold is the amount, and the rest went to fees.
/// Before the leaves are known, the whole total is the amount.
fn lightning_receive_fee_and_amount(total_sat: u64, own_sat: u64) -> (u64, u64) {
    if own_sat == 0 || own_sat > total_sat {
        return (0, total_sat);
    }
    (total_sat.saturating_sub(own_sat), own_sat)
}

/// A payment's fees and amount, from the SSP request behind its transfer.
/// Settles `status` where the request knows better than the transfer.
fn fees_and_amount(transfer: &WalletTransfer, status: &mut PaymentStatus) -> (u64, u64) {
    match transfer.user_request.clone() {
        Some(user_request) => match user_request {
            SspUserRequest::LightningSendRequest(r) => {
                // TODO: if we have the preimage it is not pending. This is a workaround
                // until spark will implement incremental syncing based on updated time.
                if r.lightning_send_payment_preimage.is_some() {
                    *status = PaymentStatus::Completed;
                }
                lightning_send_fee_and_amount(
                    transfer.total_value_sat,
                    transfer
                        .received_by(&transfer.receiver_id)
                        .unwrap_or(transfer.total_value_sat),
                    r.fee.as_sats().unwrap_or(0),
                    &r.encoded_invoice,
                )
            }
            SspUserRequest::LightningReceiveRequest(_) => lightning_receive_fee_and_amount(
                transfer.total_value_sat,
                transfer.own_share.map_or_else(
                    || {
                        transfer
                            .leaves
                            .iter()
                            .fold(0u64, |sum, leaf| sum.saturating_add(leaf.leaf.value))
                    },
                    |share| share.amount_sat,
                ),
            ),
            SspUserRequest::CoopExitRequest(r) => {
                // A committed transfer hands the funds to the provider; only
                // the provider broadcasts the payout, and only once it has
                // accepted the exit. Until then the withdrawal is still in
                // flight, however far the transfer itself got.
                *status = match r.exit_status.acceptance() {
                    CoopExitAcceptance::Accepted => *status,
                    CoopExitAcceptance::AwaitingCompletion => PaymentStatus::Pending,
                    CoopExitAcceptance::Failed => PaymentStatus::Failed,
                };
                let fee_sat = r
                    .fee
                    .as_sats()
                    .unwrap_or(0)
                    .saturating_add(r.l1_broadcast_fee.as_sats().unwrap_or(0));
                (fee_sat, transfer.total_value_sat.saturating_sub(fee_sat))
            }
            SspUserRequest::ClaimStaticDeposit(r) => {
                let fee_sat = r
                    .deposit_amount
                    .as_sats()
                    .unwrap_or(0)
                    .saturating_sub(r.credit_amount.as_sats().unwrap_or(0));
                (fee_sat, transfer.total_value_sat)
            }
            SspUserRequest::LeavesSwapRequest(_) => (0, transfer.total_value_sat),
        },
        None => (0, transfer.total_value_sat),
    }
}

fn payment_timestamp(transfer: &WalletTransfer) -> u64 {
    match transfer.created_at.map(|t| t.duration_since(UNIX_EPOCH)) {
        Some(Ok(duration)) => duration.as_secs(),
        _ => 0,
    }
}

/// A fee this wallet earned from someone else's payment, which reached it as
/// one share of a transfer with several receivers. No request describes it to
/// this wallet, so it is a plain receive of the share, settled once claimed.
fn fee_share_payment(transfer: &WalletTransfer, status: PaymentStatus) -> Option<Payment> {
    let share = transfer.own_share?;
    if transfer.direction != TransferDirection::Incoming || transfer.user_request.is_some() {
        return None;
    }
    Some(Payment {
        id: transfer.id.to_string(),
        payment_type: PaymentType::Receive,
        status,
        amount: share.amount_sat.into(),
        fees: 0,
        timestamp: payment_timestamp(transfer),
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    })
}

impl TryFrom<WalletTransfer> for Payment {
    type Error = SdkError;
    fn try_from(transfer: WalletTransfer) -> Result<Self, Self::Error> {
        if [
            TransferType::CounterSwap,
            TransferType::CounterSwapV3,
            TransferType::Swap,
            TransferType::PrimarySwapV3,
        ]
        .contains(&transfer.transfer_type)
        {
            debug!("Tried to convert swap-related transfer to payment. Transfer: {transfer:?}");
            return Err(SdkError::Generic(
                "Swap-related transfers are not considered payments".to_string(),
            ));
        }
        let payment_type = match transfer.direction {
            TransferDirection::Incoming => PaymentType::Receive,
            TransferDirection::Outgoing => PaymentType::Send,
        };
        let mut status = match transfer.status {
            TransferStatus::Completed => PaymentStatus::Completed,
            TransferStatus::SenderKeyTweaked
                if transfer.direction == TransferDirection::Outgoing =>
            {
                PaymentStatus::Completed
            }
            TransferStatus::Expired | TransferStatus::Returned => PaymentStatus::Failed,
            // A share of a fee-bearing payment is received once claimed, though
            // the transfer waits on every receiver claiming its own.
            _ if transfer.own_share.is_some_and(|share| share.claimed) => PaymentStatus::Completed,
            _ => PaymentStatus::Pending,
        };
        if let Some(payment) = fee_share_payment(&transfer, status) {
            return Ok(payment);
        }
        let (fees_sat, mut amount_sat) = fees_and_amount(&transfer, &mut status);

        let details = match PaymentDetails::from_transfer(&transfer) {
            Ok(Some(details)) => Some(details),
            Ok(None) => {
                // in case we have a completed status without user object we want
                // to keep syncing this payment
                if status == PaymentStatus::Completed
                    && [
                        TransferType::CooperativeExit,
                        TransferType::PreimageSwap,
                        TransferType::UtxoSwap,
                    ]
                    .contains(&transfer.transfer_type)
                {
                    status = PaymentStatus::Pending;
                }
                amount_sat = transfer.total_value_sat;
                None
            }
            // Failing here would stop sync at this transfer on every pass. Recording it as
            // Pending lets sync move on while reconciliation keeps re-fetching it, so the
            // details fill in once the operator or SSP serves data that maps again.
            Err(e) => {
                warn!(
                    "Recording transfer {} as pending without payment details: {e:?}",
                    transfer.id
                );
                status = PaymentStatus::Pending;
                None
            }
        };

        Ok(Payment {
            id: transfer.id.to_string(),
            payment_type,
            status,
            amount: amount_sat.into(),
            fees: fees_sat.into(),
            timestamp: payment_timestamp(&transfer),
            method: PaymentMethod::from_transfer(&transfer),
            details,
            conversion_details: None,
        })
    }
}

impl Payment {
    /// Creates a [`Payment`] from a [`LightningSendPayment`] and its associated HTLC details.
    /// `sent_sat` is everything the send's transfer carried: its fees are what it sent
    /// beyond `amount_sat`.
    ///
    /// The `htlc_details` may be stale (e.g. captured at payment creation time), so this
    /// method reconciles them with the current state of the `payment`:
    /// - The preimage is taken from `htlc_details` if present, otherwise from the payment.
    /// - If a preimage is available from either source, the HTLC status is set to
    ///   [`SparkHtlcStatus::PreimageShared`].
    pub fn from_lightning(
        payment: LightningSendPayment,
        amount_sat: u128,
        sent_sat: u128,
        transfer_id: String,
        mut htlc_details: SparkHtlcDetails,
    ) -> Result<Self, SdkError> {
        let mut status = match payment.status {
            LightningSendStatus::LightningPaymentSucceeded => PaymentStatus::Completed,
            LightningSendStatus::LightningPaymentFailed
            | LightningSendStatus::TransferFailed
            | LightningSendStatus::PreimageProvidingFailed
            | LightningSendStatus::UserSwapReturnFailed
            | LightningSendStatus::UserSwapReturned => PaymentStatus::Failed,
            _ => PaymentStatus::Pending,
        };
        if payment.payment_preimage.is_some() {
            status = PaymentStatus::Completed;
        }

        reconcile_htlc_preimage(&mut htlc_details, payment.payment_preimage.as_deref());

        let invoice_details = input::parse_invoice(&payment.encoded_invoice).ok_or(
            SdkError::Generic("Invalid invoice in LightnintSendPayment".to_string()),
        )?;
        let details = PaymentDetails::Lightning {
            description: invoice_details.description,
            invoice: payment.encoded_invoice,
            destination_pubkey: invoice_details.payee_pubkey,
            htlc_details,
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
            conversion_info: None,
        };

        Ok(Payment {
            id: transfer_id,
            payment_type: PaymentType::Send,
            status,
            amount: amount_sat,
            // The SSP's fee leaves out any partner fee the send also paid.
            fees: if sent_sat > amount_sat {
                sent_sat.saturating_sub(amount_sat)
            } else {
                payment.fee_sat.into()
            },
            timestamp: payment.created_at.cast_unsigned(),
            method: PaymentMethod::Lightning,
            details: Some(details),
            conversion_details: None,
        })
    }
}

impl From<Network> for SparkNetwork {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => SparkNetwork::Mainnet,
            Network::Regtest => SparkNetwork::Regtest,
            Network::Signet => SparkNetwork::Signet,
        }
    }
}

impl From<Fee> for spark_wallet::Fee {
    fn from(fee: Fee) -> Self {
        match fee {
            Fee::Fixed { amount } => spark_wallet::Fee::Fixed { amount },
            Fee::Rate { sat_per_vbyte } => spark_wallet::Fee::Rate { sat_per_vbyte },
        }
    }
}

impl From<spark_wallet::TokenBalance> for TokenBalance {
    fn from(value: spark_wallet::TokenBalance) -> Self {
        Self {
            balance: value.balance,
            token_metadata: value.token_metadata.into(),
        }
    }
}

impl From<spark_wallet::TokenMetadata> for TokenMetadata {
    fn from(value: spark_wallet::TokenMetadata) -> Self {
        Self {
            identifier: value.identifier,
            issuer_public_key: hex::encode(value.issuer_public_key.serialize()),
            name: value.name,
            ticker: value.ticker,
            decimals: value.decimals,
            max_supply: value.max_supply,
            is_freezable: value.is_freezable,
        }
    }
}

impl From<CoopExitFeeQuote> for SendOnchainFeeQuote {
    fn from(value: CoopExitFeeQuote) -> Self {
        Self {
            id: value.id,
            expires_at: value.expires_at,
            speed_fast: value.speed_fast.into(),
            speed_medium: value.speed_medium.into(),
            speed_slow: value.speed_slow.into(),
            is_estimate: false,
        }
    }
}

impl From<SendOnchainFeeQuote> for CoopExitFeeQuote {
    fn from(value: SendOnchainFeeQuote) -> Self {
        Self {
            id: value.id,
            expires_at: value.expires_at,
            speed_fast: value.speed_fast.into(),
            speed_medium: value.speed_medium.into(),
            speed_slow: value.speed_slow.into(),
        }
    }
}

impl From<CoopExitSpeedFeeQuote> for SendOnchainSpeedFeeQuote {
    fn from(value: CoopExitSpeedFeeQuote) -> Self {
        Self {
            user_fee_sat: value.user_fee_sat,
            l1_broadcast_fee_sat: value.l1_broadcast_fee_sat,
        }
    }
}

impl From<SendOnchainSpeedFeeQuote> for CoopExitSpeedFeeQuote {
    fn from(value: SendOnchainSpeedFeeQuote) -> Self {
        Self {
            user_fee_sat: value.user_fee_sat,
            l1_broadcast_fee_sat: value.l1_broadcast_fee_sat,
        }
    }
}

impl From<OnchainConfirmationSpeed> for ExitSpeed {
    fn from(speed: OnchainConfirmationSpeed) -> Self {
        match speed {
            OnchainConfirmationSpeed::Fast => ExitSpeed::Fast,
            OnchainConfirmationSpeed::Medium => ExitSpeed::Medium,
            OnchainConfirmationSpeed::Slow => ExitSpeed::Slow,
        }
    }
}

impl From<ExitSpeed> for OnchainConfirmationSpeed {
    fn from(speed: ExitSpeed) -> Self {
        match speed {
            ExitSpeed::Fast => OnchainConfirmationSpeed::Fast,
            ExitSpeed::Medium => OnchainConfirmationSpeed::Medium,
            ExitSpeed::Slow => OnchainConfirmationSpeed::Slow,
        }
    }
}

impl PaymentStatus {
    pub(crate) fn from_token_transaction_status(
        status: TokenTransactionStatus,
        is_transfer_transaction: bool,
    ) -> Self {
        match status {
            TokenTransactionStatus::Started
            | TokenTransactionStatus::Revealed
            | TokenTransactionStatus::Unknown => PaymentStatus::Pending,
            TokenTransactionStatus::Signed if is_transfer_transaction => PaymentStatus::Pending,
            TokenTransactionStatus::Finalized | TokenTransactionStatus::Signed => {
                PaymentStatus::Completed
            }
            TokenTransactionStatus::StartedCancelled | TokenTransactionStatus::SignedCancelled => {
                PaymentStatus::Failed
            }
        }
    }
}

impl TryFrom<PreimageRequest> for SparkHtlcDetails {
    type Error = SdkError;
    fn try_from(value: PreimageRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            payment_hash: value.payment_hash.to_string(),
            preimage: value.preimage.map(|p| p.encode_hex()),
            expiry_time: value
                .expiry_time
                .duration_since(UNIX_EPOCH)
                .map_err(|e| SdkError::Generic(format!("Invalid expiry time: {e}")))?
                .as_secs(),
            status: value.status.into(),
        })
    }
}

impl From<PreimageRequestStatus> for SparkHtlcStatus {
    fn from(status: PreimageRequestStatus) -> Self {
        match status {
            PreimageRequestStatus::WaitingForPreimage => SparkHtlcStatus::WaitingForPreimage,
            PreimageRequestStatus::PreimageShared => SparkHtlcStatus::PreimageShared,
            PreimageRequestStatus::Returned => SparkHtlcStatus::Returned,
        }
    }
}

impl From<spark_wallet::AutoOptimizationEvent> for AutoOptimizationEvent {
    fn from(value: spark_wallet::AutoOptimizationEvent) -> Self {
        match value {
            spark_wallet::AutoOptimizationEvent::Started { total_rounds } => {
                Self::Started { total_rounds }
            }
            spark_wallet::AutoOptimizationEvent::RoundCompleted {
                current_round,
                total_rounds,
            } => Self::RoundCompleted {
                current_round,
                total_rounds,
            },
            spark_wallet::AutoOptimizationEvent::Completed => Self::Completed,
            spark_wallet::AutoOptimizationEvent::Cancelled => Self::Cancelled,
            spark_wallet::AutoOptimizationEvent::Failed { error } => Self::Failed { error },
            spark_wallet::AutoOptimizationEvent::Skipped => Self::Skipped,
        }
    }
}

impl From<spark_wallet::OptimizationOutcome> for OptimizationOutcome {
    fn from(value: spark_wallet::OptimizationOutcome) -> Self {
        match value {
            spark_wallet::OptimizationOutcome::Completed { rounds_executed } => {
                Self::Completed { rounds_executed }
            }
            spark_wallet::OptimizationOutcome::InProgress => Self::InProgress,
        }
    }
}

impl From<crate::WebhookEventType> for spark_wallet::SparkWalletWebhookEventType {
    fn from(value: crate::WebhookEventType) -> Self {
        match value {
            crate::WebhookEventType::LightningReceiveFinished => {
                Self::SparkLightningReceiveFinished
            }
            crate::WebhookEventType::LightningSendFinished => Self::SparkLightningSendFinished,
            crate::WebhookEventType::CoopExitFinished => Self::SparkCoopExitFinished,
            crate::WebhookEventType::StaticDepositFinished => Self::SparkStaticDepositFinished,
            crate::WebhookEventType::Unknown(s) => Self::Unknown(s),
        }
    }
}

impl From<spark_wallet::SparkWalletWebhookEventType> for crate::WebhookEventType {
    fn from(value: spark_wallet::SparkWalletWebhookEventType) -> Self {
        match value {
            spark_wallet::SparkWalletWebhookEventType::SparkLightningReceiveFinished => {
                Self::LightningReceiveFinished
            }
            spark_wallet::SparkWalletWebhookEventType::SparkLightningSendFinished => {
                Self::LightningSendFinished
            }
            spark_wallet::SparkWalletWebhookEventType::SparkCoopExitFinished => {
                Self::CoopExitFinished
            }
            spark_wallet::SparkWalletWebhookEventType::SparkStaticDepositFinished => {
                Self::StaticDepositFinished
            }
            spark_wallet::SparkWalletWebhookEventType::Unknown(s) => Self::Unknown(s),
        }
    }
}

impl From<spark_wallet::WebhookEntry> for crate::Webhook {
    fn from(value: spark_wallet::WebhookEntry) -> Self {
        Self {
            id: value.webhook_id,
            url: value.url,
            event_types: value.event_types.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SendOnchainFeeQuote;
    use chrono::Utc;
    use macros::test_all;
    use platform_utils::time::SystemTime;
    use spark_wallet::{
        BitcoinNetwork, CoopExitFeeQuote, CoopExitSpeedFeeQuote, CurrencyAmount, CurrencyUnit,
        LightningInvoice, LightningReceiveRequest, LightningReceiveRequestStatus,
        LightningSendRequest, LightningSendRequestStatus, PublicKey, SspTransfer, Transfer,
        TransferId, TransferReceiver,
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test_all]
    fn test_a_provider_quote_is_never_an_estimate() {
        // What keeps a real quote distinguishable from a locally derived one.
        // Marking it an estimate would give it an estimate's fee headroom and
        // stop it being refreshed near expiry.
        let speed = || CoopExitSpeedFeeQuote {
            user_fee_sat: 750,
            l1_broadcast_fee_sat: 960,
        };
        let quote: SendOnchainFeeQuote = CoopExitFeeQuote {
            id: "SparkCoopExitFeeQuote:test".to_string(),
            expires_at: 1_787_907_773,
            speed_fast: speed(),
            speed_medium: speed(),
            speed_slow: speed(),
        }
        .into();

        assert!(!quote.is_estimate);
        assert_eq!(quote.id, "SparkCoopExitFeeQuote:test");
        assert_eq!(quote.expires_at, 1_787_907_773);
    }

    fn pk(fill_byte: u8) -> PublicKey {
        let mut bytes = [fill_byte; 33];
        bytes[0] = 2;
        PublicKey::from_slice(&bytes).unwrap()
    }

    /// A completed transfer created now, sent from `sender` to `receiver`.
    fn transfer(sender: PublicKey, receiver: PublicKey, transfer_type: TransferType) -> Transfer {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Transfer {
            id: TransferId::generate(),
            sender_identity_public_key: sender,
            receiver_identity_public_key: receiver,
            status: TransferStatus::Completed,
            total_value: 1_010,
            expiry_time: None,
            leaves: Vec::new(),
            created_time: Some(now),
            updated_time: Some(now),
            transfer_type,
            spark_invoice: None,
            receivers: Vec::new(),
        }
    }

    fn receiver(key: PublicKey, amount_sats: u64, completed: bool) -> TransferReceiver {
        TransferReceiver {
            identity_public_key: key,
            amount_sats,
            completed,
        }
    }

    /// A 10,200 sat payment the SSP split between the payee, `fee_payee` and
    /// itself, with only `fee_payee`'s share claimed so far.
    fn fee_bearing_transfer(ssp: PublicKey, payee: PublicKey, fee_payee: PublicKey) -> Transfer {
        let mut raw = transfer(ssp, payee, TransferType::PreimageSwap);
        raw.status = TransferStatus::SenderKeyTweaked;
        raw.total_value = 10_200;
        raw.receivers = vec![
            receiver(payee, 10_000, false),
            receiver(fee_payee, 170, true),
            receiver(ssp, 30, false),
        ];
        raw
    }

    #[test_all]
    fn a_claimed_fee_share_is_a_completed_receive_of_the_share() {
        let (ssp, payee, ours) = (pk(2), pk(7), pk(1));
        let wallet_transfer = WalletTransfer::from_transfer(
            fee_bearing_transfer(ssp, payee, ours),
            None,
            None,
            ours,
            ssp,
        );

        let payment = Payment::try_from(wallet_transfer).unwrap();

        assert_eq!(payment.payment_type, PaymentType::Receive);
        assert_eq!(payment.status, PaymentStatus::Completed);
        assert_eq!(payment.amount, 170);
        assert_eq!(payment.fees, 0);
        assert_eq!(payment.method, PaymentMethod::Spark);
    }

    #[test_all]
    fn a_fee_share_is_found_from_the_receivers_own_row_alone() {
        let (ssp, payee, ours) = (pk(2), pk(7), pk(1));
        let mut raw = fee_bearing_transfer(ssp, payee, ours);
        raw.receivers.retain(|r| r.identity_public_key == ours);
        let wallet_transfer = WalletTransfer::from_transfer(raw, None, None, ours, ssp);

        let payment = Payment::try_from(wallet_transfer).unwrap();

        assert_eq!(payment.status, PaymentStatus::Completed);
        assert_eq!(payment.amount, 170);
    }

    /// The payee's side of a fee-bearing receive another wallet attested for,
    /// such as an LNURL server: the operators keep its HTLC with the attestor.
    #[test_all]
    fn a_receive_attested_for_the_payee_completes_once_its_share_is_claimed() {
        let (ssp, ours, fee_payee) = (pk(2), pk(1), pk(7));
        let mut raw = fee_bearing_transfer(ssp, ours, fee_payee);
        raw.receivers.retain(|r| r.identity_public_key == ours);
        raw.receivers[0].completed = true;
        let amount = CurrencyAmount {
            original_value: 10_200,
            original_unit: CurrencyUnit::Satoshi,
            preferred_currency_unit: CurrencyUnit::Satoshi,
            preferred_currency_value_rounded: 10_200,
        };
        let request = LightningReceiveRequest {
            id: "request".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            network: BitcoinNetwork::Regtest,
            lightning_request_status: LightningReceiveRequestStatus::TransferCompleted,
            invoice: LightningInvoice {
                encoded_invoice: SPEC_INVOICE.to_string(),
                bitcoin_network: BitcoinNetwork::Regtest,
                payment_hash: "0001020304050607080900010203040506070809000102030405060708090102"
                    .to_string(),
                amount: amount.clone(),
                created_at: Utc::now(),
                invoice_expired_at: Utc::now(),
                memo: None,
            },
            transfer: None,
            lightning_receive_payment_preimage: Some("11".repeat(32)),
        };
        let wallet_transfer = WalletTransfer::from_transfer(
            raw,
            Some(SspTransfer {
                total_amount: amount,
                spark_id: None,
                user_request: Some(SspUserRequest::LightningReceiveRequest(request)),
            }),
            None,
            ours,
            ssp,
        );

        let payment = Payment::try_from(wallet_transfer).unwrap();

        assert_eq!(payment.status, PaymentStatus::Completed);
        assert_eq!(payment.method, PaymentMethod::Lightning);
        assert_eq!(payment.amount, 10_000);
        assert_eq!(payment.fees, 200);
        let Some(PaymentDetails::Lightning { htlc_details, .. }) = payment.details else {
            panic!("expected Lightning details, got {:?}", payment.details);
        };
        assert_eq!(htlc_details.status, SparkHtlcStatus::PreimageShared);
    }

    #[test_all]
    fn an_unclaimed_fee_share_is_pending() {
        let (ssp, payee, ours) = (pk(2), pk(7), pk(1));
        let mut raw = fee_bearing_transfer(ssp, payee, ours);
        raw.receivers[1].completed = false;
        let wallet_transfer = WalletTransfer::from_transfer(raw, None, None, ours, ssp);

        let payment = Payment::try_from(wallet_transfer).unwrap();

        assert_eq!(payment.status, PaymentStatus::Pending);
        assert_eq!(payment.amount, 170);
    }

    #[test_all]
    fn a_transfer_with_one_receiver_is_not_a_fee_share() {
        let (ours, other, ssp) = (pk(1), pk(7), pk(2));
        let mut raw = transfer(other, ours, TransferType::Transfer);
        raw.receivers = vec![receiver(ours, 1_010, true)];
        let wallet_transfer = WalletTransfer::from_transfer(raw, None, None, ours, ssp);

        assert!(wallet_transfer.own_share.is_none());
        assert_eq!(Payment::try_from(wallet_transfer).unwrap().amount, 1_010);
    }

    #[test_all]
    fn a_lightning_send_whose_details_fail_to_map_is_pending_with_its_amount() {
        let (ours, ssp) = (pk(1), pk(2));
        let request = LightningSendRequest {
            id: "request".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            network: BitcoinNetwork::Regtest,
            encoded_invoice: "not an invoice".to_string(),
            fee: CurrencyAmount {
                original_value: 10,
                original_unit: CurrencyUnit::Satoshi,
                preferred_currency_unit: CurrencyUnit::Satoshi,
                preferred_currency_value_rounded: 10,
            },
            idempotency_key: "key".to_string(),
            status: LightningSendRequestStatus::LightningPaymentSucceeded,
            transfer: None,
            lightning_send_payment_preimage: None,
        };
        let wallet_transfer = WalletTransfer::from_transfer(
            transfer(ours, ssp, TransferType::PreimageSwap),
            Some(SspTransfer {
                total_amount: request.fee.clone(),
                spark_id: None,
                user_request: Some(SspUserRequest::LightningSendRequest(request)),
            }),
            None,
            ours,
            ssp,
        );

        let payment = Payment::try_from(wallet_transfer).unwrap();

        assert!(payment.details.is_none());
        assert_eq!(payment.method, PaymentMethod::Lightning);
        assert_eq!(payment.payment_type, PaymentType::Send);
        // Pending so reconciliation re-fetches it until its details map.
        assert_eq!(payment.status, PaymentStatus::Pending);
        assert_eq!(payment.amount, 1_000);
        assert_eq!(payment.fees, 10);
    }

    #[test_all]
    fn a_spark_transfer_with_an_invalid_invoice_is_a_pending_payment() {
        let (ours, other, ssp) = (pk(1), pk(7), pk(2));
        let mut raw = transfer(other, ours, TransferType::Transfer);
        raw.spark_invoice = Some("not an invoice".to_string());
        let wallet_transfer = WalletTransfer::from_transfer(raw, None, None, ours, ssp);

        let payment = Payment::try_from(wallet_transfer).unwrap();

        assert!(payment.details.is_none());
        assert_eq!(payment.method, PaymentMethod::Spark);
        assert_eq!(payment.payment_type, PaymentType::Receive);
        assert_eq!(payment.status, PaymentStatus::Pending);
        assert_eq!(payment.amount, 1_010);
    }

    /// BOLT11's specification example: 2500u, so `250_000` sats.
    const SPEC_INVOICE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";

    // A fee-bearing send also pays partner fee payees, which the SSP's own fee
    // does not cover: everything sent beyond the invoice amount is fees.
    #[test_all]
    fn a_send_pays_the_invoice_amount_and_the_rest_is_fees() {
        assert_eq!(
            lightning_send_fee_and_amount(252_000, 251_000, 1_500, SPEC_INVOICE),
            (2_000, 250_000)
        );
        // Unquoted, the result is unchanged.
        assert_eq!(
            lightning_send_fee_and_amount(251_500, 251_500, 1_500, SPEC_INVOICE),
            (1_500, 250_000)
        );
    }

    #[test_all]
    fn an_amountless_send_pays_what_the_ssp_received_less_its_fee() {
        // Unquoted, the SSP receives everything.
        assert_eq!(
            lightning_send_fee_and_amount(10_500, 10_500, 500, "not an invoice"),
            (500, 10_000)
        );
        // Quoted, the partner fee payees receive the rest: 3_068 sent, the SSP
        // received 3_017 of it and kept 17.
        assert_eq!(
            lightning_send_fee_and_amount(3_068, 3_017, 17, "not an invoice"),
            (68, 3_000)
        );
    }

    #[test_all]
    fn a_receive_keeps_its_own_leaves_and_the_rest_is_fees() {
        assert_eq!(
            lightning_receive_fee_and_amount(102_000, 100_000),
            (2_000, 100_000)
        );
        assert_eq!(
            lightning_receive_fee_and_amount(100_000, 100_000),
            (0, 100_000)
        );
        // No leaves known yet: the total stands in.
        assert_eq!(lightning_receive_fee_and_amount(100_000, 0), (0, 100_000));
    }
}
