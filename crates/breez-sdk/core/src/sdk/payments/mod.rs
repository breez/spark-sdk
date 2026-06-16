use spark_wallet::LightningReceivePayment;
use tracing::instrument;

use crate::{
    ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse, FetchConversionLimitsRequest,
    FetchConversionLimitsResponse, GetPaymentRequest, GetPaymentResponse, WaitForPaymentIdentifier,
    error::SdkError,
    models::{
        ListPaymentsRequest, ListPaymentsResponse, Payment, PaymentRequest,
        PrepareSendPaymentRequest, PrepareSendPaymentResponse, ReceivePaymentRequest,
        ReceivePaymentResponse, SendPaymentRequest, SendPaymentResponse,
    },
    utils::payments::get_payment_with_conversion_details,
};

use super::BreezSdk;

pub(in crate::sdk) mod conversion;
mod polling;
pub(in crate::sdk) mod prepare;
mod receive;
pub(in crate::sdk) mod send;
pub(in crate::sdk) mod validation;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> Result<ReceivePaymentResponse, SdkError> {
        receive::receive_payment(self, request).await
    }

    pub async fn claim_htlc_payment(
        &self,
        request: ClaimHtlcPaymentRequest,
    ) -> Result<ClaimHtlcPaymentResponse, SdkError> {
        receive::claim_htlc_payment(self, request).await
    }

    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        // Cross-chain has its own request type (no parse step required) — early-dispatch
        // before falling through to the generic `Input` path.
        if let PaymentRequest::CrossChain {
            ref address,
            ref route,
            max_slippage_bps,
            target_overpay_bps,
        } = request.payment_request
        {
            let amount = request.amount.ok_or(SdkError::InvalidInput(
                "Amount is required for cross-chain sends".to_string(),
            ))?;
            return prepare::cross_chain::prepare(
                self,
                address,
                route,
                amount,
                request.token_identifier.clone(),
                request.conversion_options.clone(),
                request.fee_policy.unwrap_or_default(),
                max_slippage_bps,
                target_overpay_bps,
            )
            .await;
        }
        prepare::prepare(self, request).await
    }

    #[instrument(
        level = "info",
        target = "breez_sdk_core::send_payment",
        skip_all,
        fields(payment_id = tracing::field::Empty),
    )]
    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        self.maybe_ensure_spark_private_mode_initialized().await?;
        if let Some(key) = request.idempotency_key.as_deref() {
            tracing::Span::current().record("payment_id", key);
        }
        Box::pin(send::orchestrate_send(self, request, false, None)).await
    }

    pub async fn fetch_conversion_limits(
        &self,
        request: FetchConversionLimitsRequest,
    ) -> Result<FetchConversionLimitsResponse, SdkError> {
        self.token_converter
            .fetch_limits(&request)
            .await
            .map_err(Into::into)
    }

    /// Runs one pass of the pending-conversion refunder.
    ///
    /// Iterates over payments whose conversions failed and have a refund
    /// pending, then attempts to refund each one. This is the same logic the
    /// SDK runs internally on a periodic schedule when
    /// `background_tasks_enabled` is `true`. When background tasks are
    /// disabled the periodic refunder does not run, and this method is the
    /// explicit entry point for driving the pass; when background tasks are
    /// enabled, it can be called to force an immediate refund pass.
    pub async fn refund_pending_conversions(&self) -> Result<(), SdkError> {
        self.token_converter
            .refund_pending()
            .await
            .map_err(Into::into)
    }

    /// Lists payments from the storage with pagination
    ///
    /// This method provides direct access to the payment history stored in the database.
    /// It returns payments in reverse chronological order (newest first).
    ///
    /// # Arguments
    ///
    /// * `request` - Contains pagination parameters (offset and limit)
    ///
    /// # Returns
    ///
    /// * `Ok(ListPaymentsResponse)` - Contains the list of payments if successful
    /// * `Err(SdkError)` - If there was an error accessing the storage
    pub async fn list_payments(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, SdkError> {
        use crate::utils::conversions::extract_conversion_info;
        use crate::utils::payments::build_conversions;

        let mut payments = self.storage.list_payments(request.into()).await?;

        // Query child payments for payments that have conversion_details set (AMM)
        let parent_ids: Vec<String> = payments
            .iter()
            .filter(|p| p.conversion_details.is_some())
            .map(|p| p.id.clone())
            .collect();

        let related_payments_map = if parent_ids.is_empty() {
            std::collections::HashMap::default()
        } else {
            self.storage.get_payments_by_parent_ids(parent_ids).await?
        };

        for payment in &mut payments {
            let has_conversion_details = payment.conversion_details.is_some();
            let has_crosschain_info = extract_conversion_info(payment.details.clone())
                .is_some_and(|info| !matches!(info, crate::ConversionInfo::Amm { .. }));

            if !has_conversion_details && !has_crosschain_info {
                continue;
            }

            let child_payments = if has_conversion_details {
                related_payments_map.get(&payment.id).map(Vec::as_slice)
            } else {
                None
            };

            let conversions = build_conversions(payment, child_payments);

            if !conversions.is_empty() {
                if let Some(ref mut cd) = payment.conversion_details {
                    cd.conversions = conversions;
                } else {
                    let status = extract_conversion_info(payment.details.clone())
                        .map_or(crate::ConversionStatus::Completed, |info| {
                            info.status().clone()
                        });
                    payment.conversion_details = Some(crate::models::ConversionDetails {
                        status,
                        conversions,
                    });
                }
            }
        }

        Ok(ListPaymentsResponse { payments })
    }

    pub async fn get_payment(
        &self,
        request: GetPaymentRequest,
    ) -> Result<GetPaymentResponse, SdkError> {
        let payment =
            get_payment_with_conversion_details(request.payment_id, self.storage.clone()).await?;

        Ok(GetPaymentResponse { payment })
    }
}

// Private payment methods
impl BreezSdk {
    pub(crate) async fn receive_bolt11_invoice(
        &self,
        description: String,
        amount_sats: Option<u64>,
        expiry_secs: Option<u32>,
        payment_hash: Option<String>,
    ) -> Result<ReceivePaymentResponse, SdkError> {
        receive::receive_bolt11_invoice(self, description, amount_sats, expiry_secs, payment_hash)
            .await
    }

    pub(crate) async fn receive_bolt11_invoice_inner(
        &self,
        description: String,
        amount_sats: Option<u64>,
        expiry_secs: Option<u32>,
        payment_hash: Option<String>,
    ) -> Result<LightningReceivePayment, SdkError> {
        receive::receive_bolt11_invoice_inner(
            self,
            description,
            amount_sats,
            expiry_secs,
            payment_hash,
        )
        .await
    }

    pub(crate) async fn wait_for_incoming_payment(
        &self,
        identifier: WaitForPaymentIdentifier,
        completion_timeout_secs: u32,
    ) -> Result<Payment, SdkError> {
        polling::wait_for_incoming_payment(self, identifier, completion_timeout_secs).await
    }

    pub(crate) async fn finalize_payment(&self, payment: Payment) -> bool {
        polling::finalize_payment(self, payment).await
    }
}
