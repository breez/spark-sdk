use bitcoin::hashes::sha256;
use bitcoin::secp256k1::PublicKey;
use breez_sdk_common::input;
use platform_utils::time::Duration;
use platform_utils::time::SystemTime;
use platform_utils::tokio;
use spark_wallet::{ExitSpeed, SparkAddress, TransferId, TransferTokenOutput};
use spark_wallet::{InvoiceDescription, Preimage};
use std::str::FromStr;
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{Instrument, error, info, warn};

use crate::{
    BitcoinAddressDetails, Bolt11InvoiceDetails, ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse,
    ConversionEstimate, ConversionOptions, ConversionPurpose, ConversionType, FeePolicy,
    FetchConversionLimitsRequest, FetchConversionLimitsResponse, GetPaymentRequest,
    GetPaymentResponse, InputType, OnchainConfirmationSpeed, PaymentStatus, SendOnchainFeeQuote,
    SendPaymentMethod, SendPaymentOptions, SparkHtlcOptions, SparkInvoiceDetails,
    WaitForPaymentIdentifier,
    cross_chain::{CrossChainPrepared, CrossChainRoutePair},
    error::SdkError,
    events::SdkEvent,
    models::{
        ConversionStatus, ListPaymentsRequest, ListPaymentsResponse, Payment, PaymentDetails,
        PaymentRequest, PrepareSendPaymentRequest, PrepareSendPaymentResponse,
        ReceivePaymentMethod, ReceivePaymentRequest, ReceivePaymentResponse, SendPaymentRequest,
        SendPaymentResponse, conversion_steps_from_payments,
    },
    persist::PaymentMetadata,
    token_conversion::{
        ConversionAmount, DEFAULT_CONVERSION_TIMEOUT_SECS, TokenConversionResponse,
    },
    utils::{
        payments::{get_payment_and_emit_event, get_payment_with_conversion_details},
        send_payment_validation::{get_dust_limit_sats, validate_prepare_send_payment_request},
        token::map_and_persist_token_transaction,
    },
};

use super::{
    BreezSdk, SyncType,
    helpers::{InternalEventListener, get_deposit_address, is_payment_match},
};

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn receive_payment(
        &self,
        request: ReceivePaymentRequest,
    ) -> Result<ReceivePaymentResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        match request.payment_method {
            ReceivePaymentMethod::SparkAddress => Ok(ReceivePaymentResponse {
                fee: 0,
                payment_request: self
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
                let invoice = self
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
                        sender_public_key.map(|key| PublicKey::from_str(&key).unwrap()),
                    )
                    .await?;
                Ok(ReceivePaymentResponse {
                    fee: 0,
                    payment_request: invoice,
                })
            }
            ReceivePaymentMethod::BitcoinAddress { new_address } => {
                let address =
                    get_deposit_address(&self.spark_wallet, new_address.unwrap_or(false)).await?;
                Ok(ReceivePaymentResponse {
                    payment_request: address,
                    fee: 0,
                })
            }
            ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats,
                expiry_secs,
                payment_hash,
            } => {
                self.receive_bolt11_invoice(description, amount_sats, expiry_secs, payment_hash)
                    .await
            }
        }
    }

    pub async fn claim_htlc_payment(
        &self,
        request: ClaimHtlcPaymentRequest,
    ) -> Result<ClaimHtlcPaymentResponse, SdkError> {
        let preimage = Preimage::from_hex(&request.preimage)
            .map_err(|_| SdkError::InvalidInput("Invalid preimage".to_string()))?;
        let payment_hash = preimage.compute_hash();

        // Check if there is a claimable HTLC with the given payment hash
        let claimable_htlc_transfers = self
            .spark_wallet
            .list_claimable_htlc_transfers(None)
            .await?;
        if !claimable_htlc_transfers
            .iter()
            .filter_map(|t| t.htlc_preimage_request.as_ref())
            .any(|p| p.payment_hash == payment_hash)
        {
            return Err(SdkError::InvalidInput(
                "No claimable HTLC with the given payment hash".to_string(),
            ));
        }

        let transfer = self.spark_wallet.claim_htlc(&preimage).await?;
        let payment: Payment = transfer.try_into()?;

        // Insert the payment into storage to make it immediately available for listing
        self.storage.insert_payment(payment.clone()).await?;

        Ok(ClaimHtlcPaymentResponse { payment })
    }

    #[allow(clippy::too_many_lines)]
    pub async fn prepare_send_payment(
        &self,
        request: PrepareSendPaymentRequest,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        let fee_policy = request.fee_policy.unwrap_or_default();
        let token_identifier = request.token_identifier.clone();

        // Handle structured CrossChain variant directly — no parse step needed.
        if let PaymentRequest::CrossChain {
            ref address,
            ref route,
        } = request.payment_request
        {
            let amount = request.amount.ok_or(SdkError::InvalidInput(
                "Amount is required for cross-chain sends".to_string(),
            ))?;

            return self
                .prepare_cross_chain_send(address, route, amount, token_identifier, fee_policy)
                .await;
        }

        // Input string path — parse and dispatch as before.
        let PaymentRequest::Input(ref input_str) = request.payment_request else {
            return Err(SdkError::InvalidInput(
                "Expected PaymentRequest::Input".to_string(),
            ));
        };

        let parsed_input = self.parse(input_str).await?;

        validate_prepare_send_payment_request(
            &parsed_input,
            &request,
            &self.spark_wallet.get_identity_public_key().to_string(),
        )?;

        match &parsed_input {
            InputType::SparkAddress(spark_address_details) => {
                let amount = request
                    .amount
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;

                let (amount, conversion_estimate) = self
                    .resolve_send_amount_with_conversion_estimate(
                        request.conversion_options.as_ref(),
                        request.token_identifier.as_ref(),
                        amount,
                        fee_policy,
                    )
                    .await?;

                // For ToBitcoin conversions, the output is sats — clear token_identifier
                // so it reflects the output denomination, not the input.
                let is_to_bitcoin = matches!(
                    conversion_estimate,
                    Some(ConversionEstimate {
                        options: ConversionOptions {
                            conversion_type: ConversionType::ToBitcoin { .. },
                            ..
                        },
                        ..
                    })
                );
                let response_token_identifier = if is_to_bitcoin {
                    None
                } else {
                    token_identifier.clone()
                };

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkAddress {
                        address: spark_address_details.address.clone(),
                        fee: 0,
                        token_identifier: response_token_identifier.clone(),
                    },
                    amount,
                    token_identifier: response_token_identifier,
                    conversion_estimate,
                    fee_policy,
                })
            }
            InputType::SparkInvoice(spark_invoice_details) => {
                // Use request's token_identifier if provided, otherwise fall back to invoice's
                let effective_token_identifier =
                    token_identifier.or_else(|| spark_invoice_details.token_identifier.clone());

                let amount = spark_invoice_details
                    .amount
                    .or(request.amount)
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;

                let (amount, conversion_estimate) = self
                    .resolve_send_amount_with_conversion_estimate(
                        request.conversion_options.as_ref(),
                        effective_token_identifier.as_ref(),
                        amount,
                        fee_policy,
                    )
                    .await?;

                let is_to_bitcoin = matches!(
                    conversion_estimate,
                    Some(ConversionEstimate {
                        options: ConversionOptions {
                            conversion_type: ConversionType::ToBitcoin { .. },
                            ..
                        },
                        ..
                    })
                );
                let response_token_identifier = if is_to_bitcoin {
                    None
                } else {
                    effective_token_identifier.clone()
                };

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkInvoice {
                        spark_invoice_details: spark_invoice_details.clone(),
                        fee: 0,
                        token_identifier: response_token_identifier.clone(),
                    },
                    amount,
                    token_identifier: response_token_identifier,
                    conversion_estimate,
                    fee_policy,
                })
            }
            InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
                let spark_address: Option<SparkAddress> =
                    self.spark_wallet.extract_spark_address(input_str)?;

                let spark_transfer_fee_sats = if spark_address.is_some() {
                    Some(0)
                } else {
                    None
                };

                if let Some(response) = self
                    .maybe_prepare_bolt11_from_token_conversion(
                        &request,
                        detailed_bolt11_invoice,
                        spark_transfer_fee_sats,
                        token_identifier.as_ref(),
                        fee_policy,
                    )
                    .await?
                {
                    return Ok(response);
                }

                let amount = request
                    .amount
                    .or(detailed_bolt11_invoice
                        .amount_msat
                        .map(|msat| u128::from(msat).saturating_div(1000)))
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;

                // For FeesIncluded, estimate fee for user's full amount
                let lightning_fee_sats = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(input_str, Some(amount.try_into()?))
                    .await?;

                // Validate receiver amount is positive for FeesIncluded
                if fee_policy == FeePolicy::FeesIncluded
                    && detailed_bolt11_invoice.amount_msat.is_none()
                {
                    let amount_u64: u64 = amount.try_into()?;
                    if amount_u64 <= lightning_fee_sats {
                        return Err(SdkError::InvalidInput(
                            "Amount too small to cover fees".to_string(),
                        ));
                    }
                }

                let conversion_estimate = self
                    .estimate_conversion(
                        request.conversion_options.as_ref(),
                        token_identifier.as_ref(),
                        ConversionAmount::MinAmountOut(
                            amount.saturating_add(u128::from(lightning_fee_sats)),
                        ),
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: detailed_bolt11_invoice.clone(),
                        spark_transfer_fee_sats,
                        lightning_fee_sats,
                    },
                    amount,
                    token_identifier,
                    conversion_estimate,
                    fee_policy,
                })
            }
            InputType::BitcoinAddress(withdrawal_address) => {
                if let Some(response) = self
                    .maybe_prepare_bitcoin_from_token_conversion(
                        &request,
                        withdrawal_address,
                        token_identifier.as_ref(),
                        fee_policy,
                    )
                    .await?
                {
                    return Ok(response);
                }

                let amount = request
                    .amount
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;

                // Validate the amount meets the dust limit before making any network calls.
                // For FeesIncluded the output will be smaller after fees, but if the total
                // amount is already below dust there's no point fetching a fee quote.
                let dust_limit_sats = get_dust_limit_sats(&withdrawal_address.address)?;
                let amount_u64: u64 = amount.try_into()?;
                if amount_u64 < dust_limit_sats {
                    return Err(SdkError::InvalidInput(format!(
                        "Amount is below the minimum of {dust_limit_sats} sats required for this address"
                    )));
                }

                // When stable balance is active (has an active label), sats come
                // from token conversion so they don't exist yet — pass None to
                // skip leaf selection.
                let stable_balance_active = match &self.stable_balance {
                    Some(sb) => sb.get_active_label().await.is_some(),
                    None => false,
                };
                let fee_quote_amount = if stable_balance_active {
                    None
                } else {
                    Some(amount.try_into()?)
                };
                let fee_quote: SendOnchainFeeQuote = self
                    .spark_wallet
                    .fetch_coop_exit_fee_quote(&withdrawal_address.address, fee_quote_amount)
                    .await?
                    .into();

                // For FeesIncluded, validate the output after fees using the best case
                // (slow/lowest fee). Only reject if even the cheapest option results in dust.
                if fee_policy == FeePolicy::FeesIncluded {
                    let min_fee_sats = fee_quote.speed_slow.total_fee_sat();
                    let output_amount_sats = amount_u64.saturating_sub(min_fee_sats);
                    if output_amount_sats < dust_limit_sats {
                        return Err(SdkError::InvalidInput(format!(
                            "Amount is below the minimum of {dust_limit_sats} sats required for this address after lowest fees of {min_fee_sats} sats"
                        )));
                    }
                }

                // For conversion estimate, use fast fee as worst case
                let conversion_estimate = self
                    .estimate_conversion(
                        request.conversion_options.as_ref(),
                        token_identifier.as_ref(),
                        ConversionAmount::MinAmountOut(
                            amount.saturating_add(u128::from(fee_quote.speed_fast.total_fee_sat())),
                        ),
                    )
                    .await?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::BitcoinAddress {
                        address: withdrawal_address.clone(),
                        fee_quote,
                    },
                    amount,
                    token_identifier,
                    conversion_estimate,
                    fee_policy,
                })
            }
            InputType::CrossChainAddress(_) => {
                // Cross-chain addresses detected via parse() require route
                // selection. The caller should use get_cross_chain_routes()
                // to discover routes, then PaymentRequest::CrossChain with
                // the selected route.
                Err(SdkError::InvalidInput(
                    "Cross-chain address detected. Use get_cross_chain_routes() to discover \
                     routes, then PaymentRequest::CrossChain { address, route }."
                        .to_string(),
                ))
            }
            _ => Err(SdkError::InvalidInput(
                "Unsupported payment method".to_string(),
            )),
        }
    }

    pub async fn send_payment(
        &self,
        request: SendPaymentRequest,
    ) -> Result<SendPaymentResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        Box::pin(self.maybe_convert_token_send_payment(request, false, None)).await
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
        let mut payments = self.storage.list_payments(request.into()).await?;

        // Only query child payments for payments that have conversion_details set
        let parent_ids: Vec<String> = payments
            .iter()
            .filter(|p| p.conversion_details.is_some())
            .map(|p| p.id.clone())
            .collect();

        if !parent_ids.is_empty() {
            let related_payments_map = self.storage.get_payments_by_parent_ids(parent_ids).await?;

            for payment in &mut payments {
                if let Some(related_payments) = related_payments_map.get(&payment.id) {
                    match conversion_steps_from_payments(related_payments) {
                        Ok((from, to)) => {
                            if let Some(ref mut cd) = payment.conversion_details {
                                cd.from = from;
                                cd.to = to;
                            }
                        }
                        Err(e) => {
                            warn!("Failed to build conversion steps: {e}");
                        }
                    }
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
        let invoice = if let Some(payment_hash_hex) = payment_hash {
            let hash = sha256::Hash::from_str(&payment_hash_hex)
                .map_err(|e| SdkError::InvalidInput(format!("Invalid payment hash: {e}")))?;
            self.spark_wallet
                .create_hodl_lightning_invoice(
                    amount_sats.unwrap_or_default(),
                    Some(InvoiceDescription::Memo(description.clone())),
                    hash,
                    None,
                    expiry_secs,
                )
                .await?
                .invoice
        } else {
            self.spark_wallet
                .create_lightning_invoice(
                    amount_sats.unwrap_or_default(),
                    Some(InvoiceDescription::Memo(description.clone())),
                    None,
                    expiry_secs,
                    self.config.prefer_spark_over_lightning,
                )
                .await?
                .invoice
        };
        Ok(ReceivePaymentResponse {
            payment_request: invoice,
            fee: 0,
        })
    }

    /// Prepare a cross-chain send using the given route.
    async fn prepare_cross_chain_send(
        &self,
        address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        token_identifier: Option<String>,
        fee_policy: FeePolicy,
    ) -> Result<PrepareSendPaymentResponse, SdkError> {
        // Validate address is a recognized cross-chain address.
        if input::detect_address_family(address).is_none() {
            return Err(SdkError::InvalidInput(
                "Address is not a recognized cross-chain address".to_string(),
            ));
        }

        let service = self.cross_chain_providers.get(route.provider)?;

        let prepared = service
            .prepare(address, route, amount, token_identifier.clone(), None)
            .await?;

        Ok(PrepareSendPaymentResponse {
            payment_method: SendPaymentMethod::CrossChainAddress {
                route: prepared.pair,
                recipient_address: prepared.recipient_address,
                quote_id: prepared.quote_id,
                deposit_request: prepared.deposit_request,
                amount_in: prepared.amount_in,
                estimated_out: prepared.estimated_out,
                fee_amount: prepared.fee_amount,
                fee_bps: prepared.fee_bps,
                expires_at: prepared.expires_at,
            },
            amount,
            token_identifier,
            conversion_estimate: None,
            fee_policy,
        })
    }

    pub(super) async fn maybe_convert_token_send_payment(
        &self,
        request: SendPaymentRequest,
        mut suppress_payment_event: bool,
        amount_override: Option<u64>,
    ) -> Result<SendPaymentResponse, SdkError> {
        let token_identifier = request.prepare_response.token_identifier.clone();

        // Check the idempotency key is valid and payment doesn't already exist
        if request.idempotency_key.is_some() && token_identifier.is_some() {
            return Err(SdkError::InvalidInput(
                "Idempotency key is not supported for token payments".to_string(),
            ));
        }
        if let Some(idempotency_key) = &request.idempotency_key {
            // If an idempotency key is provided, check if a payment with that id already exists
            if let Ok(payment) = self
                .storage
                .get_payment_by_id(idempotency_key.clone())
                .await
            {
                return Ok(SendPaymentResponse { payment });
            }
        }
        let conversion_estimate = request.prepare_response.conversion_estimate.clone();
        // Perform the send payment, with conversion if requested
        let res = if let Some(ConversionEstimate {
            options: conversion_options,
            ..
        }) = &conversion_estimate
        {
            Box::pin(self.convert_token_send_payment_internal(
                conversion_options,
                &request,
                amount_override,
                &mut suppress_payment_event,
            ))
            .await
        } else {
            Box::pin(self.send_payment_internal(&request, amount_override)).await
        };
        // Emit payment status event and trigger wallet state sync
        if let Ok(response) = &res {
            if !suppress_payment_event {
                // Emit the payment with metadata already included
                self.event_emitter
                    .emit(&SdkEvent::from_payment(response.payment.clone()))
                    .await;
            }
            self.sync_coordinator
                .trigger_sync_no_wait(SyncType::WalletState, true)
                .await;
        }
        res
    }

    async fn convert_token_send_payment_internal(
        &self,
        conversion_options: &ConversionOptions,
        request: &SendPaymentRequest,
        caller_amount_override: Option<u64>,
        suppress_payment_event: &mut bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Suppress auto-convert while this send-with-conversion is in flight
        let _payment_guard = match &self.stable_balance {
            Some(sb) => Some(sb.acquire_payment_guard().await),
            None => None,
        };

        // Step 1: Execute the token conversion
        let (conversion_response, conversion_purpose, uses_amount_in) = self
            .execute_pre_send_conversion(conversion_options, request)
            .await?;

        // Step 2: Early-link conversion children (self-transfer only)
        self.pre_link_conversion_children(&conversion_response, &conversion_purpose)
            .await?;

        // Step 3: Trigger sync, wait for conversion, then send
        self.complete_conversion_and_send(
            conversion_options,
            &conversion_response,
            &conversion_purpose,
            request,
            uses_amount_in,
            caller_amount_override,
            suppress_payment_event,
        )
        .await
        // _payment_guard drops here, releasing the lock and waking the conversion worker
    }

    /// Executes the token conversion for the given payment method.
    ///
    /// Returns the conversion response, purpose (self-transfer vs ongoing payment),
    /// and whether the conversion used `AmountIn` (needed by `complete_conversion_and_send`
    /// to compute the amount override).
    ///
    /// Chooses the conversion direction based on whether the prepare used `AmountIn`
    /// (user specified token amount, `amount == estimate.amount_out`) or `MinAmountOut`
    /// (user specified sats). For `MinAmountOut`, the per-payment-method paths expand to
    /// `MinAmountOut(amount + fees)` so the converter delivers enough to cover the send.
    #[allow(clippy::too_many_lines)]
    async fn execute_pre_send_conversion(
        &self,
        conversion_options: &ConversionOptions,
        request: &SendPaymentRequest,
    ) -> Result<(TokenConversionResponse, ConversionPurpose, bool), SdkError> {
        let amount = request.prepare_response.amount;

        // Extract from_token_identifier from conversion options for ToBitcoin conversions.
        let from_token_identifier = match &conversion_options.conversion_type {
            ConversionType::ToBitcoin {
                from_token_identifier,
            } => Some(from_token_identifier.clone()),
            ConversionType::FromBitcoin => None,
        };

        // AmountIn vs MinAmountOut at convert time:
        // For AmountIn (user specified token amount), the prepare's `amount` is
        // derived from `estimate.amount_out` (+ optional sat balance), so
        // `amount >= estimate.amount_out`.
        // For MinAmountOut (user specified sats), the converter guarantees
        // `estimate.amount_out >= amount`, so `amount <= estimate.amount_out`
        // (strictly less when there's conversion slack).
        let uses_amount_in = request
            .prepare_response
            .conversion_estimate
            .as_ref()
            .is_some_and(|e| amount >= e.amount_out);
        let conversion_amount = if uses_amount_in {
            let token_amount = request
                .prepare_response
                .conversion_estimate
                .as_ref()
                .map(|e| e.amount_in)
                .ok_or(SdkError::InvalidInput(
                    "Conversion estimate required for token conversion".to_string(),
                ))?;
            ConversionAmount::AmountIn(token_amount)
        } else {
            ConversionAmount::MinAmountOut(amount)
        };

        match &request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress { address, .. } => {
                let spark_address = address
                    .parse::<SparkAddress>()
                    .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
                let purpose = if spark_address.identity_public_key
                    == self.spark_wallet.get_identity_public_key()
                {
                    ConversionPurpose::SelfTransfer
                } else {
                    ConversionPurpose::OngoingPayment {
                        payment_request: address.clone(),
                    }
                };
                let response = self
                    .token_converter
                    .convert(
                        conversion_options,
                        &purpose,
                        from_token_identifier.as_ref(),
                        conversion_amount,
                        None,
                    )
                    .await?;
                Ok((response, purpose, uses_amount_in))
            }
            SendPaymentMethod::SparkInvoice {
                spark_invoice_details:
                    SparkInvoiceDetails {
                        identity_public_key,
                        invoice,
                        ..
                    },
                ..
            } => {
                let own_identity_public_key =
                    self.spark_wallet.get_identity_public_key().to_string();
                let purpose = if identity_public_key == &own_identity_public_key {
                    ConversionPurpose::SelfTransfer
                } else {
                    ConversionPurpose::OngoingPayment {
                        payment_request: invoice.clone(),
                    }
                };
                let response = self
                    .token_converter
                    .convert(
                        conversion_options,
                        &purpose,
                        from_token_identifier.as_ref(),
                        conversion_amount,
                        None,
                    )
                    .await?;
                Ok((response, purpose, uses_amount_in))
            }
            SendPaymentMethod::Bolt11Invoice {
                spark_transfer_fee_sats,
                lightning_fee_sats,
                invoice_details,
                ..
            } => {
                let purpose = ConversionPurpose::OngoingPayment {
                    payment_request: invoice_details.invoice.bolt11.clone(),
                };
                let conversion_amount_override = match &conversion_amount {
                    ConversionAmount::AmountIn(_) => Some(conversion_amount),
                    ConversionAmount::MinAmountOut(_) => None,
                };
                let response = self
                    .convert_token_for_bolt11_invoice(
                        conversion_options,
                        *spark_transfer_fee_sats,
                        *lightning_fee_sats,
                        request,
                        &purpose,
                        amount,
                        from_token_identifier.as_ref(),
                        conversion_amount_override,
                    )
                    .await?;
                Ok((response, purpose, uses_amount_in))
            }
            SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                let purpose = ConversionPurpose::OngoingPayment {
                    payment_request: address.address.clone(),
                };
                let conversion_amount_override = match &conversion_amount {
                    ConversionAmount::AmountIn(_) => Some(conversion_amount),
                    ConversionAmount::MinAmountOut(_) => None,
                };
                let response = self
                    .convert_token_for_bitcoin_address(
                        conversion_options,
                        fee_quote,
                        request,
                        &purpose,
                        amount,
                        from_token_identifier.as_ref(),
                        conversion_amount_override,
                    )
                    .await?;
                Ok((response, purpose, uses_amount_in))
            }
            SendPaymentMethod::CrossChainAddress { .. } => {
                // Cross-chain sends bypass the AMM token converter entirely;
                // they should never reach pre-send conversion.
                Err(SdkError::InvalidInput(
                    "Cross-chain sends do not support AMM conversions".to_string(),
                ))
            }
        }
    }

    /// Links conversion child payments to their parent to hide them from `list_payments`.
    ///
    /// Only self-transfers are linked immediately (parent is the conversion receive, known upfront).
    /// All other cases are deferred until after the actual send completes.
    async fn pre_link_conversion_children(
        &self,
        conversion_response: &TokenConversionResponse,
        conversion_purpose: &ConversionPurpose,
    ) -> Result<(), SdkError> {
        if *conversion_purpose == ConversionPurpose::SelfTransfer {
            self.storage
                .insert_payment_metadata(
                    conversion_response.sent_payment_id.clone(),
                    PaymentMetadata {
                        parent_payment_id: Some(conversion_response.received_payment_id.clone()),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Ok(())
    }

    /// Waits for conversion to complete, then sends the actual payment.
    ///
    /// For self-transfers, returns immediately after conversion completes.
    /// For ongoing payments, sends the actual payment and links any remaining children.
    /// For `AmountIn` conversions, computes `amount_override = converted_sats + sats_change`
    /// where `sats_change` is the difference between the prepare amount and the estimated
    /// conversion output (representing any existing sat balance included at prepare time).
    /// If `caller_amount_override` is provided (e.g. from the LNURL flow which handles
    /// its own fee logic), it takes precedence over the computed override.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn complete_conversion_and_send(
        &self,
        conversion_options: &ConversionOptions,
        conversion_response: &TokenConversionResponse,
        conversion_purpose: &ConversionPurpose,
        request: &SendPaymentRequest,
        uses_amount_in: bool,
        caller_amount_override: Option<u64>,
        suppress_payment_event: &mut bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Trigger a wallet state sync if converting from Bitcoin to token
        if matches!(
            conversion_options.conversion_type,
            ConversionType::FromBitcoin
        ) {
            self.sync_coordinator
                .trigger_sync_no_wait(SyncType::WalletState, true)
                .await;
        }

        // Wait for the received conversion payment to complete
        let payment = self
            .wait_for_payment(
                WaitForPaymentIdentifier::PaymentId(
                    conversion_response.received_payment_id.clone(),
                ),
                conversion_options
                    .completion_timeout_secs
                    .unwrap_or(DEFAULT_CONVERSION_TIMEOUT_SECS),
            )
            .await
            .map_err(|e| {
                SdkError::Generic(format!("Timeout waiting for conversion to complete: {e}"))
            })?;

        // For self-transfers, suppress the event and return
        if *conversion_purpose == ConversionPurpose::SelfTransfer {
            *suppress_payment_event = true;
            return Ok(SendPaymentResponse { payment });
        }

        // Determine the amount to use for the actual send.
        //
        // If the caller provided an amount_override (e.g. LNURL flow with its own
        // fee logic), use it directly.
        //
        // For AmountIn conversions (user specified token amount), the prepare's
        // `amount` includes estimated conversion output + any existing sat balance
        // (sats_change). At send time, use the actual converted sats + sats_change
        // to honor the prepare estimate while accounting for slippage.
        // This unifies send-all (sats_change > 0) and non-send-all (sats_change = 0).
        //
        // For MinAmountOut conversions (user specified sats, e.g. auto stable
        // balance), the conversion guarantees ≥ requested sats, so no override is
        // needed — send exactly the prepared amount.
        let amount_override = if let Some(override_amount) = caller_amount_override {
            tracing::trace!(
                override_amount,
                "complete_conversion_and_send: using caller-provided amount_override"
            );
            Some(override_amount)
        } else if uses_amount_in {
            let converted_sats: u64 = payment
                .amount
                .try_into()
                .map_err(|_| SdkError::Generic("Converted sats too large for u64".to_string()))?;
            let estimated_conversion_out: u64 = request
                .prepare_response
                .conversion_estimate
                .as_ref()
                .map_or(0, |e| e.amount_out)
                .try_into()
                .map_err(|_| SdkError::Generic("Estimated sats too large for u64".to_string()))?;
            let sats_change = request
                .prepare_response
                .amount
                .try_into()
                .map(|amount: u64| amount.saturating_sub(estimated_conversion_out))
                .unwrap_or(0);
            let total = converted_sats.saturating_add(sats_change);
            tracing::trace!(
                converted_sats,
                estimated_conversion_out,
                sats_change,
                total,
                prepared_amount = request.prepare_response.amount,
                fee_policy = ?request.prepare_response.fee_policy,
                "complete_conversion_and_send: amount_override = converted_sats + sats_change"
            );
            Some(total)
        } else {
            tracing::trace!(
                prepared_amount = request.prepare_response.amount,
                fee_policy = ?request.prepare_response.fee_policy,
                "complete_conversion_and_send: no override (MinAmountOut conversion)"
            );
            None
        };

        // Now send the actual payment
        let response = Box::pin(self.send_payment_internal(request, amount_override)).await?;

        // Link conversion children to the send payment (deferred linking)
        self.storage
            .insert_payment_metadata(
                conversion_response.sent_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(response.payment.id.clone()),
                    ..Default::default()
                },
            )
            .await?;
        self.storage
            .insert_payment_metadata(
                conversion_response.received_payment_id.clone(),
                PaymentMetadata {
                    parent_payment_id: Some(response.payment.id.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Persist Completed status on the actual send payment
        self.storage
            .insert_payment_metadata(
                response.payment.id.clone(),
                PaymentMetadata {
                    conversion_status: Some(ConversionStatus::Completed),
                    ..Default::default()
                },
            )
            .await?;

        // Fetch the updated payment with conversion details
        get_payment_with_conversion_details(response.payment.id, self.storage.clone())
            .await
            .map(|payment| SendPaymentResponse { payment })
    }

    pub(super) async fn send_payment_internal(
        &self,
        request: &SendPaymentRequest,
        amount_override: Option<u64>,
    ) -> Result<SendPaymentResponse, SdkError> {
        let amount = request.prepare_response.amount;
        let token_identifier = request.prepare_response.token_identifier.clone();

        match &request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress { address, .. } => {
                Box::pin(self.send_spark_address(
                    address,
                    token_identifier,
                    amount_override.map_or(amount, u128::from),
                    request.options.as_ref(),
                    request.idempotency_key.clone(),
                ))
                .await
            }
            SendPaymentMethod::SparkInvoice {
                spark_invoice_details,
                ..
            } => {
                self.send_spark_invoice(
                    &spark_invoice_details.invoice,
                    request,
                    amount_override.map_or(amount, u128::from),
                )
                .await
            }
            SendPaymentMethod::Bolt11Invoice {
                invoice_details,
                spark_transfer_fee_sats,
                lightning_fee_sats,
                ..
            } => {
                Box::pin(self.send_bolt11_invoice(
                    invoice_details,
                    *spark_transfer_fee_sats,
                    *lightning_fee_sats,
                    request,
                    amount_override,
                    amount,
                ))
                .await
            }
            SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                self.send_bitcoin_address(address, fee_quote, request, amount_override)
                    .await
            }
            SendPaymentMethod::CrossChainAddress {
                route,
                recipient_address,
                quote_id,
                deposit_request,
                amount_in,
                estimated_out,
                fee_amount,
                fee_bps,
                expires_at,
            } => {
                let service = self.cross_chain_providers.get(route.provider)?;

                let prepared = CrossChainPrepared {
                    quote_id: quote_id.clone(),
                    deposit_request: deposit_request.clone(),
                    amount_in: *amount_in,
                    estimated_out: *estimated_out,
                    fee_amount: *fee_amount,
                    fee_bps: *fee_bps,
                    expires_at: expires_at.clone(),
                    pair: route.clone(),
                    recipient_address: recipient_address.clone(),
                    token_identifier: token_identifier.clone(),
                };

                let response = service.send(&prepared).await?;

                // The Spark transfer leg produces a payment row via the
                // existing wallet event path. The provider's send() has
                // already attached conversion metadata to it. The payment
                // row may not be immediately available if async sync hasn't
                // processed the Spark transfer yet.
                match self
                    .storage
                    .get_payment_by_id(response.payment_id.clone())
                    .await
                {
                    Ok(p) => Ok(SendPaymentResponse { payment: p }),
                    Err(_) => Err(SdkError::Generic(format!(
                        "Cross-chain send submitted (order {}), but payment row not yet synced.",
                        response.order_id
                    ))),
                }
            }
        }
    }

    async fn send_spark_address(
        &self,
        address: &str,
        token_identifier: Option<String>,
        amount: u128,
        options: Option<&SendPaymentOptions>,
        idempotency_key: Option<String>,
    ) -> Result<SendPaymentResponse, SdkError> {
        let spark_address = address
            .parse::<SparkAddress>()
            .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;

        // If HTLC options are provided, send an HTLC transfer
        if let Some(SendPaymentOptions::SparkAddress { htlc_options }) = options
            && let Some(htlc_options) = htlc_options
        {
            if token_identifier.is_some() {
                return Err(SdkError::InvalidInput(
                    "Can't provide both token identifier and HTLC options".to_string(),
                ));
            }

            return self
                .send_spark_htlc(
                    &spark_address,
                    amount.try_into()?,
                    htlc_options,
                    idempotency_key,
                )
                .await;
        }

        let payment = if let Some(identifier) = token_identifier {
            self.send_spark_token_address(identifier, amount, spark_address)
                .await?
        } else {
            let transfer_id = idempotency_key
                .as_ref()
                .map(|key| TransferId::from_str(key))
                .transpose()?;
            let transfer = self
                .spark_wallet
                .transfer(amount.try_into()?, &spark_address, transfer_id)
                .await?;
            transfer.try_into()?
        };

        // Insert the payment into storage to make it immediately available for listing
        self.storage.insert_payment(payment.clone()).await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_spark_htlc(
        &self,
        address: &SparkAddress,
        amount_sat: u64,
        htlc_options: &SparkHtlcOptions,
        idempotency_key: Option<String>,
    ) -> Result<SendPaymentResponse, SdkError> {
        let payment_hash = sha256::Hash::from_str(&htlc_options.payment_hash)
            .map_err(|_| SdkError::InvalidInput("Invalid payment hash".to_string()))?;

        if htlc_options.expiry_duration_secs == 0 {
            return Err(SdkError::InvalidInput(
                "Expiry duration must be greater than 0".to_string(),
            ));
        }
        let expiry_duration = Duration::from_secs(htlc_options.expiry_duration_secs);

        let transfer_id = idempotency_key
            .as_ref()
            .map(|key| TransferId::from_str(key))
            .transpose()?;
        let transfer = self
            .spark_wallet
            .create_htlc(
                amount_sat,
                address,
                &payment_hash,
                expiry_duration,
                transfer_id,
            )
            .await?;

        let payment: Payment = transfer.try_into()?;

        // Insert the payment into storage to make it immediately available for listing
        self.storage.insert_payment(payment.clone()).await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_spark_token_address(
        &self,
        token_identifier: String,
        amount: u128,
        receiver_address: SparkAddress,
    ) -> Result<Payment, SdkError> {
        let token_transaction = self
            .spark_wallet
            .transfer_tokens(
                vec![TransferTokenOutput {
                    token_id: token_identifier,
                    amount,
                    receiver_address: receiver_address.clone(),
                    spark_invoice: None,
                }],
                None,
                None,
            )
            .await?;

        map_and_persist_token_transaction(&self.spark_wallet, &self.storage, &token_transaction)
            .await
    }

    async fn send_spark_invoice(
        &self,
        invoice: &str,
        request: &SendPaymentRequest,
        amount: u128,
    ) -> Result<SendPaymentResponse, SdkError> {
        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|key| TransferId::from_str(key))
            .transpose()?;

        let payment = match self
            .spark_wallet
            .fulfill_spark_invoice(invoice, Some(amount), transfer_id)
            .await?
        {
            spark_wallet::FulfillSparkInvoiceResult::Transfer(wallet_transfer) => {
                (*wallet_transfer).try_into()?
            }
            spark_wallet::FulfillSparkInvoiceResult::TokenTransaction(token_transaction) => {
                map_and_persist_token_transaction(
                    &self.spark_wallet,
                    &self.storage,
                    &token_transaction,
                )
                .await?
            }
        };

        // Insert the payment into storage to make it immediately available for listing
        self.storage.insert_payment(payment.clone()).await?;

        Ok(SendPaymentResponse { payment })
    }

    /// For `FeesIncluded` + amountless Bolt11: calculates the amount to send
    /// (`receiver_amount` + any overpayment from fee decrease).
    async fn calculate_fees_included_bolt11_amount(
        &self,
        invoice: &str,
        user_amount: u64,
        stored_fee: u64,
    ) -> Result<u64, SdkError> {
        let receiver_amount = user_amount.saturating_sub(stored_fee);
        if receiver_amount == 0 {
            return Err(SdkError::InvalidInput(
                "Amount too small to cover fees".to_string(),
            ));
        }

        // Re-estimate current fee for receiver amount
        let current_fee = self
            .spark_wallet
            .fetch_lightning_send_fee_estimate(invoice, Some(receiver_amount))
            .await?;

        // If current fee exceeds stored fee, fail
        if current_fee > stored_fee {
            return Err(SdkError::Generic(
                "Fee increased since prepare. Please retry.".to_string(),
            ));
        }

        // Calculate overpayment
        let overpayment = stored_fee.saturating_sub(current_fee);

        // Protect against excessive fee overpayment.
        // Allow overpayment up to 100% of actual fee, with a minimum of 1 sat.
        let max_allowed_overpayment = current_fee.max(1);
        if overpayment > max_allowed_overpayment {
            return Err(SdkError::Generic(format!(
                "Fee overpayment ({overpayment} sats) exceeds allowed maximum ({max_allowed_overpayment} sats)"
            )));
        }

        if overpayment > 0 {
            info!(
                overpayment_sats = overpayment,
                stored_fee_sats = stored_fee,
                current_fee_sats = current_fee,
                "FeesIncluded fee overpayment applied for Bolt11"
            );
        }

        Ok(receiver_amount.saturating_add(overpayment))
    }

    async fn send_bolt11_invoice(
        &self,
        invoice_details: &Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
        request: &SendPaymentRequest,
        amount_override: Option<u64>,
        amount: u128,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Determine routing preference and actual fee before calculating the send amount,
        // so FeesIncluded deducts the correct fee (Spark=0 vs Lightning).
        let (prefer_spark, completion_timeout_secs) = match request.options {
            Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark,
                completion_timeout_secs,
            }) => (prefer_spark, completion_timeout_secs),
            _ => (self.config.prefer_spark_over_lightning, None),
        };
        let is_spark_route = prefer_spark && spark_transfer_fee_sats.is_some();
        let fee_sats = if is_spark_route {
            spark_transfer_fee_sats.unwrap_or(0)
        } else {
            lightning_fee_sats
        };

        // Handle FeesIncluded: deduct fees from the total balance.
        // Applies to both amountless invoices and fixed-amount invoices with amount_override
        // (send-all-with-conversion via LNURL — overpays the invoice to drain the wallet).
        let is_fees_included = request.prepare_response.fee_policy == FeePolicy::FeesIncluded;
        let amount_to_send = if is_fees_included
            && (invoice_details.amount_msat.is_none() || amount_override.is_some())
        {
            let total_sats: u64 = match amount_override {
                Some(sat_balance) => sat_balance,
                None => amount.try_into()?,
            };
            // Spark route: deduct known fee directly (often 0).
            // Lightning route: re-estimate fees via calculate_fees_included_bolt11_amount
            // which handles fee changes between prepare and send.
            let amt = if is_spark_route {
                total_sats.saturating_sub(fee_sats)
            } else {
                self.calculate_fees_included_bolt11_amount(
                    &invoice_details.invoice.bolt11,
                    total_sats,
                    fee_sats,
                )
                .await?
            };
            Some(u128::from(amt))
        } else {
            match amount_override {
                Some(amt) => Some(amt.into()),
                None => match invoice_details.amount_msat {
                    Some(_) => None,
                    None => Some(amount),
                },
            }
        };
        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|idempotency_key| TransferId::from_str(idempotency_key))
            .transpose()?;

        let payment_response = Box::pin(
            self.spark_wallet.pay_lightning_invoice(
                &invoice_details.invoice.bolt11,
                amount_to_send
                    .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                    .transpose()?,
                Some(fee_sats),
                prefer_spark,
                transfer_id,
            ),
        )
        .await?;
        let payment = match payment_response.lightning_payment {
            Some(lightning_payment) => {
                let ssp_id = lightning_payment.id.clone();
                let htlc_details = payment_response
                    .transfer
                    .htlc_preimage_request
                    .ok_or_else(|| {
                        SdkError::Generic(
                            "Missing HTLC details for Lightning send payment".to_string(),
                        )
                    })?
                    .try_into()?;
                let payment = Payment::from_lightning(
                    lightning_payment,
                    amount,
                    payment_response.transfer.id.to_string(),
                    htlc_details,
                )?;
                self.poll_lightning_send_payment(&payment, ssp_id);
                payment
            }
            None => payment_response.transfer.try_into()?,
        };

        let completion_timeout_secs = completion_timeout_secs.unwrap_or(0);

        if completion_timeout_secs == 0 {
            // Insert the payment into storage to make it immediately available for listing
            self.storage.insert_payment(payment.clone()).await?;

            return Ok(SendPaymentResponse { payment });
        }

        let payment = self
            .wait_for_payment(
                WaitForPaymentIdentifier::PaymentId(payment.id.clone()),
                completion_timeout_secs,
            )
            .await
            .unwrap_or(payment);

        // Insert the payment into storage to make it immediately available for listing
        self.storage.insert_payment(payment.clone()).await?;

        Ok(SendPaymentResponse { payment })
    }

    async fn send_bitcoin_address(
        &self,
        address: &BitcoinAddressDetails,
        fee_quote: &SendOnchainFeeQuote,
        request: &SendPaymentRequest,
        amount_override: Option<u64>,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Extract confirmation speed from options
        let confirmation_speed = match &request.options {
            Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                confirmation_speed.clone()
            }
            None => OnchainConfirmationSpeed::Fast, // Default to fast
            _ => {
                return Err(SdkError::InvalidInput(
                    "Invalid options for Bitcoin address payment".to_string(),
                ));
            }
        };

        let exit_speed: ExitSpeed = confirmation_speed.clone().into();

        // Calculate fee based on selected speed
        let fee_sats = match confirmation_speed {
            OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
            OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
            OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
        };

        // Compute amount - for FeesIncluded, receiver gets total minus fees.
        // amount_override (send-all post-conversion) is always FeesIncluded.
        let total_sats: u64 =
            amount_override.unwrap_or(request.prepare_response.amount.try_into()?);
        let amount_sats = if request.prepare_response.fee_policy == FeePolicy::FeesIncluded {
            total_sats.saturating_sub(fee_sats)
        } else {
            total_sats
        };

        // Validate the output amount meets the dust limit for this address type
        let dust_limit_sats = get_dust_limit_sats(&address.address)?;
        if amount_sats < dust_limit_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount is below the minimum of {dust_limit_sats} sats required for this address"
            )));
        }

        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|idempotency_key| TransferId::from_str(idempotency_key))
            .transpose()?;
        let response = self
            .spark_wallet
            .withdraw(
                &address.address,
                Some(amount_sats),
                exit_speed,
                fee_quote.clone().into(),
                transfer_id,
            )
            .await?;

        let payment: Payment = response.try_into()?;

        self.storage.insert_payment(payment.clone()).await?;

        Ok(SendPaymentResponse { payment })
    }

    pub(super) async fn wait_for_payment(
        &self,
        identifier: WaitForPaymentIdentifier,
        completion_timeout_secs: u32,
    ) -> Result<Payment, SdkError> {
        let (tx, mut rx) = mpsc::channel(20);
        // Use internal listener to see raw events before middleware processing.
        // This is critical because TokenConversionMiddleware suppresses conversion
        // child events, but wait_for_payment needs to see them.
        let id = self
            .event_emitter
            .add_internal_listener(Box::new(InternalEventListener::new(tx)))
            .await;

        // Run the main logic in a closure so cleanup always happens,
        // even if an early `?` exits (e.g. get_payment_by_invoice failure).
        let result = async {
            // First check if we already have the completed payment in storage
            let payment = match &identifier {
                WaitForPaymentIdentifier::PaymentId(payment_id) => self
                    .storage
                    .get_payment_by_id(payment_id.clone())
                    .await
                    .ok(),
                WaitForPaymentIdentifier::PaymentRequest(payment_request) => {
                    self.storage
                        .get_payment_by_invoice(payment_request.clone())
                        .await?
                }
            };
            if let Some(payment) = payment
                && payment.status == PaymentStatus::Completed
            {
                return Ok(payment);
            }

            timeout(Duration::from_secs(completion_timeout_secs.into()), async {
                loop {
                    let Some(event) = rx.recv().await else {
                        return Err(SdkError::Generic("Event channel closed".to_string()));
                    };

                    let SdkEvent::PaymentSucceeded { payment } = event else {
                        continue;
                    };

                    if is_payment_match(&payment, &identifier) {
                        return Ok(payment);
                    }
                }
            })
            .await
            .map_err(|_| SdkError::Generic("Timeout waiting for payment".to_string()))?
        }
        .await;

        self.event_emitter.remove_internal_listener(&id).await;
        result
    }

    // Pools the lightning send payment until it is in completed state.
    fn poll_lightning_send_payment(&self, payment: &Payment, ssp_id: String) {
        const MAX_POLL_ATTEMPTS: u32 = 20;
        let payment_id = payment.id.clone();
        info!("Polling lightning send payment {}", payment_id);

        let Some(htlc_details) = payment.details.as_ref().and_then(|d| match d {
            PaymentDetails::Lightning { htlc_details, .. } => Some(htlc_details.clone()),
            _ => None,
        }) else {
            error!(
                "Missing HTLC details for lightning send payment {payment_id}, skipping polling"
            );
            return;
        };
        let spark_wallet = self.spark_wallet.clone();
        let storage = self.storage.clone();
        let sync_coordinator = self.sync_coordinator.clone();
        let event_emitter = self.event_emitter.clone();
        let payment = payment.clone();
        let payment_id = payment_id.clone();
        let mut shutdown = self.shutdown_sender.subscribe();
        let span = tracing::Span::current();

        tokio::spawn(async move {
            for i in 0..MAX_POLL_ATTEMPTS {
                info!(
                    "Polling lightning send payment {} attempt {}",
                    payment_id, i
                );
                select! {
                    _ = shutdown.changed() => {
                        info!("Shutdown signal received");
                        return;
                    },
                    p = spark_wallet.fetch_lightning_send_payment(&ssp_id) => {
                        if let Ok(Some(p)) = p && let Ok(payment) = Payment::from_lightning(p.clone(), payment.amount, payment.id.clone(), htlc_details.clone()) {
                            info!("Polling payment status = {} {:?}", payment.status, p.status);
                            if payment.status != PaymentStatus::Pending {
                                info!("Polling payment completed status = {}", payment.status);
                                // Update storage before emitting event so that
                                // get_payment returns the correct status immediately.
                                if let Err(e) = storage.insert_payment(payment.clone()).await {
                                    error!("Failed to update payment in storage: {e:?}");
                                }
                                // Fetch the payment to include already stored metadata
                                get_payment_and_emit_event(&storage, &event_emitter, payment.clone()).await;
                                sync_coordinator
                                    .trigger_sync_no_wait(SyncType::WalletState, true)
                                    .await;
                                return;
                            }
                        }

                        let sleep_time = if i < 5 {
                            Duration::from_secs(1)
                        } else {
                            Duration::from_secs(i.into())
                        };
                        tokio::time::sleep(sleep_time).await;
                    }
                }
            }
        }.instrument(span));
    }

    #[expect(clippy::too_many_arguments)]
    async fn convert_token_for_bolt11_invoice(
        &self,
        conversion_options: &ConversionOptions,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
        request: &SendPaymentRequest,
        conversion_purpose: &ConversionPurpose,
        amount: u128,
        token_identifier: Option<&String>,
        conversion_amount_override: Option<ConversionAmount>,
    ) -> Result<TokenConversionResponse, SdkError> {
        let conversion_amount = if let Some(ca) = conversion_amount_override {
            ca
        } else {
            // Determine the fee to be used based on preference
            let fee_sats = match request.options {
                Some(SendPaymentOptions::Bolt11Invoice { prefer_spark, .. }) => {
                    match (prefer_spark, spark_transfer_fee_sats) {
                        (true, Some(fee)) => fee,
                        _ => lightning_fee_sats,
                    }
                }
                _ => lightning_fee_sats,
            };
            // The absolute minimum amount out is the lightning invoice amount plus fee
            let min_amount_out = amount.saturating_add(u128::from(fee_sats));
            ConversionAmount::MinAmountOut(min_amount_out)
        };

        self.token_converter
            .convert(
                conversion_options,
                conversion_purpose,
                token_identifier,
                conversion_amount,
                None,
            )
            .await
            .map_err(Into::into)
    }

    /// Gets conversion options for a payment, auto-populating from stable balance config if needed.
    ///
    /// Returns the provided options if set, or auto-populates from stable balance config
    /// if configured and there's not enough sats balance to cover the payment.
    async fn get_conversion_options_for_payment(
        &self,
        options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        payment_amount: u128,
    ) -> Result<Option<ConversionOptions>, SdkError> {
        if let Some(stable_balance) = &self.stable_balance {
            stable_balance
                .get_conversion_options(options, token_identifier, payment_amount)
                .await
                .map_err(Into::into)
        } else {
            Ok(options.cloned())
        }
    }

    /// Estimates a conversion for a payment, returning `None` when no conversion is needed.
    ///
    /// For `AmountIn`: validates with the given options directly (caller knows what to convert).
    /// For `MinAmountOut`: auto-populates conversion options from stable balance config when applicable.
    pub(super) async fn estimate_conversion(
        &self,
        request_options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        conversion_amount: ConversionAmount,
    ) -> Result<Option<ConversionEstimate>, SdkError> {
        match conversion_amount {
            ConversionAmount::AmountIn(_) => self
                .token_converter
                .validate(request_options, token_identifier, conversion_amount)
                .await
                .map_err(Into::into),
            ConversionAmount::MinAmountOut(amount) => {
                let options = self
                    .get_conversion_options_for_payment(request_options, token_identifier, amount)
                    .await?;
                self.token_converter
                    .validate(options.as_ref(), token_identifier, conversion_amount)
                    .await
                    .map_err(Into::into)
            }
        }
    }

    // Returns `(is_token_conversion, is_send_all)`.
    pub(super) async fn is_token_conversion(
        &self,
        conversion_options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount: Option<u128>,
        fee_policy: FeePolicy,
    ) -> Result<(bool, bool), SdkError> {
        let (
            Some(amount),
            Some(ConversionOptions {
                conversion_type:
                    ConversionType::ToBitcoin {
                        from_token_identifier,
                    },
                ..
            }),
        ) = (amount, conversion_options)
        else {
            return Ok((false, false));
        };

        // If the caller passed a token_identifier it must match conversion options.
        // If they omitted it, we can't compare against the balance, so is_send_all=false.
        // Send-all also requires stable balance to be active with a matching active token,
        // otherwise we shouldn't sweep the existing sat balance.
        let is_send_all = match token_identifier {
            Some(token_id) => {
                if token_id != from_token_identifier {
                    return Err(SdkError::Generic(
                        "Request token identifier must match conversion options".to_string(),
                    ));
                }
                let token_balances = self.spark_wallet.get_token_balances().await?;
                let token_balance = token_balances.get(token_id).map_or(0, |tb| tb.balance);
                let has_active_stable_token = match &self.stable_balance {
                    Some(sb) => sb.get_active_token_identifier().await.as_ref() == Some(token_id),
                    None => false,
                };
                amount == token_balance
                    && fee_policy == FeePolicy::FeesIncluded
                    && has_active_stable_token
            }
            None => false,
        };

        Ok((true, is_send_all))
    }

    /// Estimates the sats available for a send that may go through a token→BTC conversion.
    ///
    /// Branches on `token_identifier`:
    /// - **Set** → `amount` is in token base units; uses `AmountIn(amount)` (variable
    ///   sat output). For send-all, adds the existing sat balance to the conversion
    ///   output.
    /// - **Not set** → `amount` is already in sats; uses `MinAmountOut(amount)` so
    ///   the converter is guaranteed to deliver at least `amount` sats or fail.
    ///   `estimated_sats == amount` in this case.
    ///
    /// Returns `(estimated_sats, conversion_estimate)`. When the request is not a
    /// token conversion, `estimated_sats == amount` and `conversion_estimate` is None,
    /// so callers can use `conversion_estimate.is_some()` to detect the conversion path.
    /// The returned `estimated_sats` is the *raw* expected conversion output — callers
    /// that need a defensive lower bound (e.g. LNURL invoice sizing on the `AmountIn`
    /// path) should apply their own slippage buffer.
    pub(super) async fn estimate_sats_from_token_conversion(
        &self,
        conversion_options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount: u128,
        fee_policy: FeePolicy,
    ) -> Result<(u128, Option<ConversionEstimate>), SdkError> {
        let (is_token_conversion, is_send_all) = self
            .is_token_conversion(
                conversion_options,
                token_identifier,
                Some(amount),
                fee_policy,
            )
            .await?;
        if !is_token_conversion {
            return Ok((amount, None));
        }

        // When token_identifier is provided, `amount` is in token units → AmountIn.
        // When it's omitted, `amount` is in sats → MinAmountOut (we want at least
        // that many sats out of the conversion).
        let (conversion_amount, estimated_sats_from_conversion) = if token_identifier.is_some() {
            let estimate = self
                .estimate_conversion(
                    conversion_options,
                    token_identifier,
                    ConversionAmount::AmountIn(amount),
                )
                .await?;
            let sats = estimate.as_ref().map_or(0, |e| e.amount_out);
            (estimate, sats)
        } else {
            let estimate = self
                .estimate_conversion(
                    conversion_options,
                    token_identifier,
                    ConversionAmount::MinAmountOut(amount),
                )
                .await?;
            // For MinAmountOut, the requested sats is the amount we asked for.
            (estimate, amount)
        };

        // For send-all, include existing sats balance — the actual send at execution
        // time will use the full post-conversion balance.
        let estimated_sats = if is_send_all {
            let sat_balance = u128::from(self.spark_wallet.get_balance().await?);
            estimated_sats_from_conversion.saturating_add(sat_balance)
        } else {
            estimated_sats_from_conversion
        };

        Ok((estimated_sats, conversion_amount))
    }

    /// Resolves the effective send amount and conversion estimate for a prepare flow
    /// where the destination accepts sats directly (Spark address, Spark invoice).
    ///
    /// - **Token conversion** (`token_identifier` set + `ToBitcoin` options): substitutes
    ///   `amount` with the post-conversion estimated sats, returns the `AmountIn` estimate.
    /// - **Plain send with conversion options** (no `token_identifier`, sats `amount` +
    ///   options): keeps `amount` as-is, attaches a `MinAmountOut` estimate for display.
    /// - **Plain send (no options)**: passes through unchanged with `None` estimate.
    async fn resolve_send_amount_with_conversion_estimate(
        &self,
        conversion_options: Option<&ConversionOptions>,
        token_identifier: Option<&String>,
        amount: u128,
        fee_policy: FeePolicy,
    ) -> Result<(u128, Option<ConversionEstimate>), SdkError> {
        let (estimated_sats, conversion_estimate) = self
            .estimate_sats_from_token_conversion(
                conversion_options,
                token_identifier,
                amount,
                fee_policy,
            )
            .await?;
        if conversion_estimate.is_some() {
            return Ok((estimated_sats, conversion_estimate));
        }
        let estimate = self
            .estimate_conversion(
                conversion_options,
                token_identifier,
                ConversionAmount::MinAmountOut(amount),
            )
            .await?;
        Ok((amount, estimate))
    }

    /// Prepares a Bolt11 invoice payment for token-to-Bitcoin conversion (send-all
    /// or non-send-all). Returns `Ok(None)` when the request is not a token conversion
    /// so the caller can fall through to the regular bolt11 prepare path.
    ///
    /// Estimates the conversion, fetches lightning fees based on the estimated sats,
    /// and validates the receiver amount covers fees.
    async fn maybe_prepare_bolt11_from_token_conversion(
        &self,
        request: &PrepareSendPaymentRequest,
        invoice: &Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        token_identifier: Option<&String>,
        fee_policy: FeePolicy,
    ) -> Result<Option<PrepareSendPaymentResponse>, SdkError> {
        let Some(token_amount) = request.amount else {
            return Ok(None);
        };
        let (estimated_sats, conversion_estimate) = self
            .estimate_sats_from_token_conversion(
                request.conversion_options.as_ref(),
                token_identifier,
                token_amount,
                fee_policy,
            )
            .await?;
        if conversion_estimate.is_none() {
            return Ok(None);
        }

        let input_str = match &request.payment_request {
            PaymentRequest::Input(s) => s.as_str(),
            PaymentRequest::CrossChain { .. } => {
                return Err(SdkError::InvalidInput(
                    "Token conversion is not supported for cross-chain sends".to_string(),
                ));
            }
        };
        let lightning_fee_sats = self
            .spark_wallet
            .fetch_lightning_send_fee_estimate(input_str, Some(estimated_sats.try_into()?))
            .await?;

        let total_u64: u64 = estimated_sats.try_into()?;
        // For fixed-amount invoices, the converted sats must cover invoice amount + fees.
        // For amountless invoices (send-all), just check fees are covered.
        let min_required = if let Some(amount_msat) = invoice.amount_msat {
            (amount_msat / 1000).saturating_add(lightning_fee_sats)
        } else {
            lightning_fee_sats
        };
        if total_u64 <= min_required {
            return Err(SdkError::InvalidInput(
                "Token conversion amount too small to cover invoice amount and fees".to_string(),
            ));
        }

        Ok(Some(PrepareSendPaymentResponse {
            payment_method: SendPaymentMethod::Bolt11Invoice {
                invoice_details: invoice.clone(),
                spark_transfer_fee_sats,
                lightning_fee_sats,
            },
            amount: estimated_sats,
            // ToBitcoin conversion outputs sats — token_identifier is None
            token_identifier: None,
            conversion_estimate,
            fee_policy,
        }))
    }

    /// Prepares a Bitcoin address payment for token-to-Bitcoin conversion (send-all
    /// or non-send-all). Returns `Ok(None)` when the request is not a token conversion
    /// so the caller can fall through to the regular bitcoin address prepare path.
    ///
    /// Estimates the conversion, fetches onchain fee quote based on the estimated
    /// sats, and validates the output after fees meets the dust limit.
    async fn maybe_prepare_bitcoin_from_token_conversion(
        &self,
        request: &PrepareSendPaymentRequest,
        withdrawal_address: &BitcoinAddressDetails,
        token_identifier: Option<&String>,
        fee_policy: FeePolicy,
    ) -> Result<Option<PrepareSendPaymentResponse>, SdkError> {
        let Some(token_amount) = request.amount else {
            return Ok(None);
        };
        let (estimated_sats, conversion_estimate) = self
            .estimate_sats_from_token_conversion(
                request.conversion_options.as_ref(),
                token_identifier,
                token_amount,
                fee_policy,
            )
            .await?;
        if conversion_estimate.is_none() {
            return Ok(None);
        }

        let dust_limit_sats = get_dust_limit_sats(&withdrawal_address.address)?;
        let total_u64: u64 = estimated_sats.try_into()?;
        if total_u64 < dust_limit_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount is below the minimum of {dust_limit_sats} sats required for this address"
            )));
        }

        // Pass None for amount — the sats don't exist yet (still tokens),
        // so leaf selection would fail. Get a generic fee estimate instead.
        let fee_quote: SendOnchainFeeQuote = self
            .spark_wallet
            .fetch_coop_exit_fee_quote(&withdrawal_address.address, None)
            .await?
            .into();

        let min_fee_sats = fee_quote.speed_slow.total_fee_sat();
        let output_amount_sats = total_u64.saturating_sub(min_fee_sats);
        if output_amount_sats < dust_limit_sats {
            return Err(SdkError::InvalidInput(format!(
                "Amount is below the minimum of {dust_limit_sats} sats required for this address after lowest fees of {min_fee_sats} sats"
            )));
        }

        Ok(Some(PrepareSendPaymentResponse {
            payment_method: SendPaymentMethod::BitcoinAddress {
                address: withdrawal_address.clone(),
                fee_quote,
            },
            amount: estimated_sats,
            // ToBitcoin conversion outputs sats — token_identifier is None
            token_identifier: None,
            conversion_estimate,
            fee_policy,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    async fn convert_token_for_bitcoin_address(
        &self,
        conversion_options: &ConversionOptions,
        fee_quote: &SendOnchainFeeQuote,
        request: &SendPaymentRequest,
        conversion_purpose: &ConversionPurpose,
        amount: u128,
        token_identifier: Option<&String>,
        conversion_amount_override: Option<ConversionAmount>,
    ) -> Result<TokenConversionResponse, SdkError> {
        let conversion_amount = if let Some(ca) = conversion_amount_override {
            ca
        } else {
            // Derive fee_sats from request.options confirmation speed
            let fee_sats = match &request.options {
                Some(SendPaymentOptions::BitcoinAddress { confirmation_speed }) => {
                    match confirmation_speed {
                        OnchainConfirmationSpeed::Slow => fee_quote.speed_slow.total_fee_sat(),
                        OnchainConfirmationSpeed::Medium => fee_quote.speed_medium.total_fee_sat(),
                        OnchainConfirmationSpeed::Fast => fee_quote.speed_fast.total_fee_sat(),
                    }
                }
                _ => fee_quote.speed_fast.total_fee_sat(), // Default to fast
            };
            // The absolute minimum amount out is the amount plus fee
            let min_amount_out = amount.saturating_add(u128::from(fee_sats));
            ConversionAmount::MinAmountOut(min_amount_out)
        };

        self.token_converter
            .convert(
                conversion_options,
                conversion_purpose,
                token_identifier,
                conversion_amount,
                None,
            )
            .await
            .map_err(Into::into)
    }
}
