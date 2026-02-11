use breez_sdk_common::lnurl::{self, error::LnurlError, pay::validate_lnurl_pay};
use tracing::info;

use crate::{
    FeePolicy, InputType, LnurlAuthRequestDetails, LnurlCallbackStatus, LnurlPayInfo,
    LnurlPayRequest, LnurlPayResponse, LnurlWithdrawInfo, LnurlWithdrawRequest,
    LnurlWithdrawResponse, PrepareLnurlPayRequest, PrepareLnurlPayResponse, SendPaymentMethod,
    WaitForPaymentIdentifier,
    error::SdkError,
    events::SdkEvent,
    models::{
        PrepareSendPaymentResponse, ReceivePaymentMethod, ReceivePaymentRequest, SendPaymentRequest,
    },
    persist::{ObjectCacheRepository, PaymentMetadata},
};
use breez_sdk_common::lnurl::withdraw::execute_lnurl_withdraw;

use super::{BreezSdk, helpers::process_success_action};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn prepare_lnurl_pay(
        &self,
        request: PrepareLnurlPayRequest,
    ) -> Result<PrepareLnurlPayResponse, SdkError> {
        let fee_policy = request.fee_policy.unwrap_or_default();
        let amount_sats = request.amount_sats;

        if fee_policy == FeePolicy::FeesIncluded && request.conversion_options.is_some() {
            return Err(SdkError::InvalidInput(
                "FeesIncluded cannot be combined with token conversion".to_string(),
            ));
        }

        // FeesIncluded uses the double-query approach
        if fee_policy == FeePolicy::FeesIncluded {
            return self
                .prepare_lnurl_pay_fees_included(request, amount_sats)
                .await;
        }

        let success_data = match validate_lnurl_pay(
            self.lnurl_client.as_ref(),
            amount_sats.saturating_mul(1_000),
            &request.comment,
            &request.pay_request.clone().into(),
            self.config.network.into(),
            request.validate_success_action_url,
        )
        .await?
        {
            lnurl::pay::ValidatedCallbackResponse::EndpointError { data } => {
                return Err(LnurlError::EndpointError(data.reason).into());
            }
            lnurl::pay::ValidatedCallbackResponse::EndpointSuccess { data } => data,
        };

        let prepare_response = self
            .prepare_send_payment(crate::PrepareSendPaymentRequest {
                payment_request: success_data.pr,
                amount: Some(u128::from(amount_sats)),
                token_identifier: None,
                conversion_options: request.conversion_options.clone(),
                fee_policy: None,
            })
            .await?;

        let SendPaymentMethod::Bolt11Invoice {
            invoice_details,
            lightning_fee_sats,
            ..
        } = prepare_response.payment_method
        else {
            return Err(SdkError::Generic(
                "Expected Bolt11Invoice payment method".to_string(),
            ));
        };

        Ok(PrepareLnurlPayResponse {
            amount_sats,
            comment: request.comment,
            pay_request: request.pay_request,
            invoice_details,
            fee_sats: lightning_fee_sats,
            success_action: success_data.success_action.map(From::from),
            conversion_estimate: prepare_response.conversion_estimate,
            fee_policy,
        })
    }

    #[allow(clippy::too_many_lines)]
    pub async fn lnurl_pay(&self, request: LnurlPayRequest) -> Result<LnurlPayResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;

        let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;

        // For FeesIncluded, extract amount from the invoice (set during prepare)
        let receiver_amount_sats: u64 = if is_fees_included {
            request
                .prepare_response
                .invoice_details
                .amount_msat
                .ok_or_else(|| SdkError::Generic("Missing invoice amount".to_string()))?
                / 1000
        } else {
            request.prepare_response.amount_sats
        };

        // Calculate amount override for FeesIncluded operations
        let amount_override = if is_fees_included {
            // Re-estimate current fee for the invoice
            let current_fee = self
                .spark_wallet
                .fetch_lightning_send_fee_estimate(
                    &request.prepare_response.invoice_details.invoice.bolt11,
                    None,
                )
                .await?;

            // fees_included_fee = first_fee (from prepare), which is the total we need to pay in fees
            let fees_included_fee = request.prepare_response.fee_sats;

            if current_fee > fees_included_fee {
                return Err(SdkError::Generic(
                    "Fee increased since prepare. Please retry.".to_string(),
                ));
            }

            // Overpay by the difference to respect prepared amount
            let overpayment = fees_included_fee.saturating_sub(current_fee);

            // Protect against excessive fee overpayment.
            // Allow overpayment up to 100% of actual fee, with a minimum of 1 sat.
            let max_allowed_overpayment = current_fee.max(1);
            if overpayment > max_allowed_overpayment {
                return Err(SdkError::Generic(format!(
                    "Fee overpayment ({overpayment} sats) exceeds allowed maximum ({max_allowed_overpayment} sats)"
                )));
            }

            if overpayment > 0 {
                tracing::info!(
                    overpayment_sats = overpayment,
                    fees_included_fee_sats = fees_included_fee,
                    current_fee_sats = current_fee,
                    "FeesIncluded fee overpayment applied"
                );
            }
            Some(receiver_amount_sats.saturating_add(overpayment))
        } else {
            None
        };

        let mut payment = Box::pin(self.maybe_convert_token_send_payment(
            SendPaymentRequest {
                prepare_response: PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: request.prepare_response.invoice_details,
                        spark_transfer_fee_sats: None,
                        lightning_fee_sats: request.prepare_response.fee_sats,
                    },
                    amount: u128::from(receiver_amount_sats),
                    token_identifier: None,
                    conversion_estimate: request.prepare_response.conversion_estimate,
                    fee_policy: FeePolicy::FeesExcluded, // Always FeesExcluded for internal handling
                },
                options: None,
                idempotency_key: request.idempotency_key,
            },
            true,
            amount_override,
        ))
        .await?
        .payment;

        let success_action = process_success_action(
            &payment,
            request
                .prepare_response
                .success_action
                .clone()
                .map(Into::into)
                .as_ref(),
        )?;

        let lnurl_info = LnurlPayInfo {
            ln_address: request.prepare_response.pay_request.address,
            comment: request.prepare_response.comment,
            domain: Some(request.prepare_response.pay_request.domain),
            metadata: Some(request.prepare_response.pay_request.metadata_str),
            processed_success_action: success_action.clone().map(From::from),
            raw_success_action: request.prepare_response.success_action,
        };
        let Some(crate::PaymentDetails::Lightning {
            lnurl_pay_info,
            description,
            ..
        }) = &mut payment.details
        else {
            return Err(SdkError::Generic(
                "Expected Lightning payment details".to_string(),
            ));
        };
        *lnurl_pay_info = Some(lnurl_info.clone());

        let lnurl_description = lnurl_info.extract_description();
        description.clone_from(&lnurl_description);

        self.storage
            .insert_payment_metadata(
                payment.id.clone(),
                PaymentMetadata {
                    lnurl_pay_info: Some(lnurl_info),
                    lnurl_description,
                    ..Default::default()
                },
            )
            .await?;

        self.event_emitter
            .emit(&SdkEvent::from_payment(payment.clone()))
            .await;
        Ok(LnurlPayResponse {
            payment,
            success_action: success_action.map(From::from),
        })
    }

    /// Performs an LNURL withdraw operation for the amount of satoshis to
    /// withdraw and the LNURL withdraw request details. The LNURL withdraw request
    /// details can be obtained from calling [`BreezSdk::parse`].
    ///
    /// The method generates a Lightning invoice for the withdraw amount, stores
    /// the LNURL withdraw metadata, and performs the LNURL withdraw using  the generated
    /// invoice.
    ///
    /// If the `completion_timeout_secs` parameter is provided and greater than 0, the
    /// method will wait for the payment to be completed within that period. If the
    /// withdraw is completed within the timeout, the `payment` field in the response
    /// will be set with the payment details. If the `completion_timeout_secs`
    /// parameter is not provided or set to 0, the method will not wait for the payment
    /// to be completed. If the withdraw is not completed within the
    /// timeout, the `payment` field will be empty.
    ///
    /// # Arguments
    ///
    /// * `request` - The LNURL withdraw request
    ///
    /// # Returns
    ///
    /// Result containing either:
    /// * `LnurlWithdrawResponse` - The payment details if the withdraw request was successful
    /// * `SdkError` - If there was an error during the withdraw process
    pub async fn lnurl_withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> Result<LnurlWithdrawResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        let LnurlWithdrawRequest {
            amount_sats,
            withdraw_request,
            completion_timeout_secs,
        } = request;
        let withdraw_request: breez_sdk_common::lnurl::withdraw::LnurlWithdrawRequestDetails =
            withdraw_request.into();
        if !withdraw_request.is_amount_valid(amount_sats) {
            return Err(SdkError::InvalidInput(
                "Amount must be within min/max LNURL withdrawable limits".to_string(),
            ));
        }

        // Generate a Lightning invoice for the withdraw
        let payment_request = self
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::Bolt11Invoice {
                    description: withdraw_request.default_description.clone(),
                    amount_sats: Some(amount_sats),
                    expiry_secs: None,
                    payment_hash: None,
                },
            })
            .await?
            .payment_request;

        // Store the LNURL withdraw metadata before executing the withdraw
        let cache = ObjectCacheRepository::new(self.storage.clone());
        cache
            .save_payment_metadata(
                &payment_request,
                &PaymentMetadata {
                    lnurl_withdraw_info: Some(LnurlWithdrawInfo {
                        withdraw_url: withdraw_request.callback.clone(),
                    }),
                    lnurl_description: Some(withdraw_request.default_description.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Perform the LNURL withdraw using the generated invoice
        let withdraw_response = execute_lnurl_withdraw(
            self.lnurl_client.as_ref(),
            &withdraw_request,
            &payment_request,
        )
        .await?;
        if let lnurl::withdraw::ValidatedCallbackResponse::EndpointError { data } =
            withdraw_response
        {
            return Err(LnurlError::EndpointError(data.reason).into());
        }

        let completion_timeout_secs = match completion_timeout_secs {
            Some(secs) if secs > 0 => secs,
            _ => {
                return Ok(LnurlWithdrawResponse {
                    payment_request,
                    payment: None,
                });
            }
        };

        // Wait for the payment to be completed
        let payment = self
            .wait_for_payment(
                WaitForPaymentIdentifier::PaymentRequest(payment_request.clone()),
                completion_timeout_secs,
            )
            .await
            .ok();
        Ok(LnurlWithdrawResponse {
            payment_request,
            payment,
        })
    }

    /// Performs LNURL-auth with the service.
    ///
    /// This method implements the LNURL-auth protocol as specified in LUD-04 and LUD-05.
    /// It derives a domain-specific linking key, signs the challenge, and sends the
    /// authentication request to the service.
    pub async fn lnurl_auth(
        &self,
        request_data: LnurlAuthRequestDetails,
    ) -> Result<LnurlCallbackStatus, SdkError> {
        let request: breez_sdk_common::lnurl::auth::LnurlAuthRequestDetails = request_data.into();
        let status = breez_sdk_common::lnurl::auth::perform_lnurl_auth(
            self.lnurl_client.as_ref(),
            &request,
            self.lnurl_auth_signer.as_ref(),
        )
        .await
        .map_err(|e| match e {
            LnurlError::ServiceConnectivity(msg) => SdkError::NetworkError(msg.to_string()),
            LnurlError::InvalidUri(msg) => SdkError::InvalidInput(msg),
            _ => SdkError::Generic(e.to_string()),
        })?;
        Ok(status.into())
    }
}

// Private LNURL methods
impl BreezSdk {
    /// Prepares an LNURL pay `FeesIncluded` operation using a double-query approach.
    ///
    /// This method:
    /// 1. Validates amount doesn't exceed LNURL `max_sendable`
    /// 2. First query: gets invoice for full amount to estimate fees
    /// 3. Calculates actual send amount (amount - estimated fee)
    /// 4. Second query: gets invoice for actual amount
    /// 5. Returns the prepare response with the second invoice
    pub(super) async fn prepare_lnurl_pay_fees_included(
        &self,
        request: PrepareLnurlPayRequest,
        amount_sats: u64,
    ) -> Result<PrepareLnurlPayResponse, SdkError> {
        if amount_sats == 0 {
            return Err(SdkError::InvalidInput(
                "Amount must be greater than 0".to_string(),
            ));
        }

        // 1. Validate amount is within LNURL limits
        let min_sendable_sats = request.pay_request.min_sendable.div_ceil(1000);
        let max_sendable_sats = request.pay_request.max_sendable / 1000;

        if amount_sats < min_sendable_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount ({amount_sats} sats) is below LNURL minimum ({min_sendable_sats} sats)"
            )));
        }

        if amount_sats > max_sendable_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount ({amount_sats} sats) exceeds LNURL maximum ({max_sendable_sats} sats)"
            )));
        }

        // 2. First query: get invoice for full amount to estimate fees
        // Note: We don't intend to pay this invoice. It's only for fee estimation.
        let first_invoice = validate_lnurl_pay(
            self.lnurl_client.as_ref(),
            amount_sats.saturating_mul(1_000), // convert to msats
            &request.comment,
            &request.pay_request.clone().into(),
            self.config.network.into(),
            request.validate_success_action_url,
        )
        .await?;

        let first_data = match first_invoice {
            lnurl::pay::ValidatedCallbackResponse::EndpointError { data } => {
                return Err(LnurlError::EndpointError(data.reason).into());
            }
            lnurl::pay::ValidatedCallbackResponse::EndpointSuccess { data } => data,
        };

        // 3. Get fee estimate for first invoice
        let first_fee = self
            .spark_wallet
            .fetch_lightning_send_fee_estimate(&first_data.pr, None)
            .await?;

        // 4. Calculate actual send amount (amount - fee)
        let actual_amount = amount_sats.saturating_sub(first_fee);

        // Validate against LNURL minimum
        if actual_amount < min_sendable_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount after fees ({actual_amount} sats) is below LNURL minimum ({min_sendable_sats} sats)"
            )));
        }

        // 5. Second query: get invoice for actual amount (back-to-back, no delay)
        let success_data = match validate_lnurl_pay(
            self.lnurl_client.as_ref(),
            actual_amount.saturating_mul(1_000),
            &request.comment,
            &request.pay_request.clone().into(),
            self.config.network.into(),
            request.validate_success_action_url,
        )
        .await?
        {
            lnurl::pay::ValidatedCallbackResponse::EndpointError { data } => {
                return Err(LnurlError::EndpointError(data.reason).into());
            }
            lnurl::pay::ValidatedCallbackResponse::EndpointSuccess { data } => data,
        };

        // 6. Get actual fee for the smaller invoice
        let actual_fee = self
            .spark_wallet
            .fetch_lightning_send_fee_estimate(&success_data.pr, None)
            .await?;

        // If fee increased between queries, fail (user must retry)
        if actual_fee > first_fee {
            return Err(SdkError::Generic(
                "Fee increased between queries. Please retry.".to_string(),
            ));
        }

        // Parse the invoice to get details
        let parsed = self.parse(&success_data.pr).await?;
        let InputType::Bolt11Invoice(invoice_details) = parsed else {
            return Err(SdkError::Generic(
                "Expected Bolt11 invoice from LNURL".to_string(),
            ));
        };

        info!(
            "LNURL FeesIncluded prepared: amount={amount_sats}, receiver_amount={actual_amount}, fee={first_fee}"
        );

        Ok(PrepareLnurlPayResponse {
            amount_sats,
            comment: request.comment,
            pay_request: request.pay_request,
            invoice_details,
            fee_sats: first_fee,
            success_action: success_data.success_action.map(From::from),
            conversion_estimate: None,
            fee_policy: FeePolicy::FeesIncluded,
        })
    }
}
