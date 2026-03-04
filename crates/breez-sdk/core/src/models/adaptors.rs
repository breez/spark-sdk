use breez_sdk_common::input::{
    self, InputType, PaymentRequestSource, SparkInvoiceDetails, parse_spark_address,
};
use spark_wallet::{
    CoopExitFeeQuote, CoopExitSpeedFeeQuote, ExitSpeed, LightningSendPayment, LightningSendStatus,
    Network as SparkNetwork, PreimageRequest, PreimageRequestStatus, SspUserRequest,
    TokenTransactionStatus, TransferDirection, TransferStatus, TransferType, WalletTransfer,
};
use std::time::Duration;

use tracing::{debug, warn};
use web_time::UNIX_EPOCH;

use crate::{
    Fee, Network, OnchainConfirmationSpeed, OptimizationProgress, Payment, PaymentDetails,
    PaymentMethod, PaymentStatus, PaymentType, SdkError, SendOnchainFeeQuote,
    SendOnchainSpeedFeeQuote, SparkHtlcDetails, SparkHtlcStatus, SparkInvoicePaymentDetails,
    TokenBalance, TokenMetadata,
};

/// Feb 1, 2026 00:00:00 UTC — transfers before this may lack HTLC data on the operator.
const HTLC_DATA_REQUIRED_SINCE: Duration = Duration::from_secs(1_769_904_000);

/// Derive HTLC details from SSP request fields when the operator lacks the
/// `PreimageRequest`. Only allowed for old transfers (before [`HTLC_DATA_REQUIRED_SINCE`]);
/// new transfers without HTLC data are considered an error.
fn derive_htlc_details_from_ssp(
    transfer: &WalletTransfer,
    payment_hash: &str,
    preimage: Option<&str>,
) -> Result<SparkHtlcDetails, SdkError> {
    let cutoff = UNIX_EPOCH
        .checked_add(HTLC_DATA_REQUIRED_SINCE)
        .ok_or_else(|| SdkError::Generic("HTLC cutoff time overflow".to_string()))?;
    let is_old = transfer.created_at.is_none_or(|t| t < cutoff);
    if !is_old {
        return Err(SdkError::Generic(format!(
            "Missing HTLC details for Lightning payment transfer {}",
            transfer.id
        )));
    }

    warn!(
        "Missing HTLC preimage request for Lightning transfer {}, deriving from SSP data",
        transfer.id
    );

    let status = match transfer.status {
        TransferStatus::Completed => SparkHtlcStatus::PreimageShared,
        TransferStatus::Expired | TransferStatus::Returned => SparkHtlcStatus::Returned,
        _ => SparkHtlcStatus::WaitingForPreimage,
    };
    Ok(SparkHtlcDetails {
        payment_hash: payment_hash.to_string(),
        preimage: preimage.map(ToString::to_string),
        expiry_time: transfer
            .expiry_time
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs()),
        status,
    })
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
            SspUserRequest::ClaimStaticDeposit(request) => PaymentDetails::Deposit {
                tx_id: request.transaction_id.clone(),
            },
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
            _ => PaymentStatus::Pending,
        };
        let (fees_sat, mut amount_sat) = match transfer.clone().user_request {
            Some(user_request) => match user_request {
                SspUserRequest::LightningSendRequest(r) => {
                    // TODO: if we have the preimage it is not pending. This is a workaround
                    // until spark will implement incremental syncing based on updated time.
                    if r.lightning_send_payment_preimage.is_some() {
                        status = PaymentStatus::Completed;
                    }
                    let fee_sat = r.fee.as_sats().unwrap_or(0);
                    (fee_sat, transfer.total_value_sat.saturating_sub(fee_sat))
                }
                SspUserRequest::CoopExitRequest(r) => {
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
                _ => (0, transfer.total_value_sat),
            },
            None => (0, transfer.total_value_sat),
        };

        let details = PaymentDetails::from_transfer(&transfer)?;
        if details.is_none() {
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
        }

        Ok(Payment {
            id: transfer.id.to_string(),
            payment_type,
            status,
            amount: amount_sat.into(),
            fees: fees_sat.into(),
            timestamp: match transfer.created_at.map(|t| t.duration_since(UNIX_EPOCH)) {
                Some(Ok(duration)) => duration.as_secs(),
                _ => 0,
            },
            method: PaymentMethod::from_transfer(&transfer),
            details,
            conversion_details: None,
        })
    }
}

impl Payment {
    /// Creates a [`Payment`] from a [`LightningSendPayment`] and its associated HTLC details.
    ///
    /// The `htlc_details` may be stale (e.g. captured at payment creation time), so this
    /// method reconciles them with the current state of the `payment`:
    /// - The preimage is taken from `htlc_details` if present, otherwise from the payment.
    /// - If a preimage is available from either source, the HTLC status is set to
    ///   [`SparkHtlcStatus::PreimageShared`].
    pub fn from_lightning(
        payment: LightningSendPayment,
        amount_sat: u128,
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
        };

        Ok(Payment {
            id: transfer_id,
            payment_type: PaymentType::Send,
            status,
            amount: amount_sat,
            fees: payment.fee_sat.into(),
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

impl From<spark_wallet::OptimizationProgress> for OptimizationProgress {
    fn from(value: spark_wallet::OptimizationProgress) -> Self {
        Self {
            is_running: value.is_running,
            current_round: value.current_round,
            total_rounds: value.total_rounds,
        }
    }
}
