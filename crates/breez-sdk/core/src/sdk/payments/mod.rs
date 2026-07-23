use spark_wallet::LightningReceivePayment;
use tracing::instrument;

use crate::{
    ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse, FetchConversionLimitsRequest,
    FetchConversionLimitsResponse, GetPaymentRequest, GetPaymentResponse, WaitForPaymentIdentifier,
    error::SdkError,
    models::{
        BuildUnsignedTokenBatchPackageRequest, BuildUnsignedTransferPackageRequest,
        ListPaymentsRequest, ListPaymentsResponse, Payment, PaymentRequest,
        PrepareSendPaymentRequest, PrepareSendPaymentResponse, PrepareSendTokenBatchRequest,
        PrepareSendTokenBatchResponse, PublishSignedTransferPackageRequest,
        PublishSignedTransferPackageResponse, ReceivePaymentRequest, ReceivePaymentResponse,
        SendPaymentRequest, SendPaymentResponse, SendTokenBatchRequest, SendTokenBatchResponse,
        UnsignedTransferPackage,
    },
    utils::payments::get_payment_with_conversion_details,
};

use super::BreezSdk;

pub(in crate::sdk) mod client_signing;
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

    /// Prepares a token send to several payees, all paid by one token
    /// transaction.
    ///
    /// Each recipient is a Spark address or a Spark invoice, and one batch may
    /// span several tokens. The response resolves every invoice into the token
    /// and amount it requests, and reports what the batch debits per token.
    pub async fn prepare_send_token_batch(
        &self,
        request: PrepareSendTokenBatchRequest,
    ) -> Result<PrepareSendTokenBatchResponse, SdkError> {
        prepare::token_batch::prepare(self, request).await
    }

    /// Sends the batch prepared by [`BreezSdk::prepare_send_token_batch`], returning
    /// one payment per recipient in recipient order.
    ///
    /// Retrying after a failure that leaves the outcome unknown may pay twice:
    /// a token transfer has no idempotency key, since the operator can only be
    /// asked about a transaction by a hash that is computed while broadcasting.
    /// Look for the batch with a `Token` payment details filter on the
    /// transaction hash before sending it again.
    #[instrument(level = "info", target = "breez_sdk_core::send_token_batch", skip_all)]
    pub async fn send_token_batch(
        &self,
        request: SendTokenBatchRequest,
    ) -> Result<SendTokenBatchResponse, SdkError> {
        self.maybe_ensure_spark_private_mode_initialized().await?;
        Box::pin(send::token_batch::send(self, request)).await
    }

    /// Builds the unsigned package for the batch prepared by
    /// [`BreezSdk::prepare_send_token_batch`], for signing outside the SDK.
    ///
    /// Publish the signed package with
    /// [`BreezSdk::publish_signed_transfer_package`], which returns every payment.
    pub async fn build_unsigned_token_batch_package(
        &self,
        request: BuildUnsignedTokenBatchPackageRequest,
    ) -> Result<UnsignedTransferPackage, SdkError> {
        Box::pin(client_signing::build_unsigned_token_batch_package(
            self,
            &request.prepare_response,
        ))
        .await
    }

    pub async fn build_unsigned_transfer_package(
        &self,
        request: BuildUnsignedTransferPackageRequest,
    ) -> Result<UnsignedTransferPackage, SdkError> {
        Box::pin(client_signing::build_unsigned_transfer_package(
            self,
            &request.prepare_response,
            request.options.as_ref(),
        ))
        .await
    }

    #[instrument(
        level = "info",
        target = "breez_sdk_core::publish_signed_transfer_package",
        skip_all
    )]
    pub async fn publish_signed_transfer_package(
        &self,
        request: PublishSignedTransferPackageRequest,
    ) -> Result<PublishSignedTransferPackageResponse, SdkError> {
        self.maybe_ensure_spark_private_mode_initialized().await?;
        Box::pin(send::publish_signed_transfer_package(
            self,
            &request.signed_package,
        ))
        .await
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
