use bitcoin::hashes::sha256;
use spark_wallet::{ExitSpeed, SparkAddress, TransferId, TransferTokenOutput};
use std::str::FromStr;
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{error, info, warn};
use web_time::Duration;

use crate::{
    BitcoinAddressDetails, Bolt11InvoiceDetails, ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse,
    ConversionEstimate, ConversionOptions, ConversionPurpose, ConversionType,
    FetchConversionLimitsRequest, FetchConversionLimitsResponse, GetPaymentRequest,
    GetPaymentResponse, InputType, OnchainConfirmationSpeed, PayAmount, PaymentDetails,
    PaymentStatus, SendOnchainFeeQuote, SendPaymentMethod, SendPaymentOptions, SparkHtlcOptions,
    SparkInvoiceDetails, TokenConversionResponse, WaitForPaymentIdentifier,
    error::SdkError,
    events::SdkEvent,
    models::{
        ListPaymentsRequest, ListPaymentsResponse, Payment, PrepareSendPaymentRequest,
        PrepareSendPaymentResponse, ReceivePaymentMethod, ReceivePaymentRequest,
        ReceivePaymentResponse, SendPaymentRequest, SendPaymentResponse,
    },
    persist::{ObjectCacheRepository, PaymentMetadata, StaticDepositAddress},
    token_conversion::DEFAULT_CONVERSION_TIMEOUT_SECS,
    utils::{
        send_payment_validation::validate_prepare_send_payment_request,
        token::map_and_persist_token_transaction,
    },
};
use bitcoin::secp256k1::PublicKey;
use spark_wallet::{InvoiceDescription, Preimage};
use tokio_with_wasm::alias as tokio;
use web_time::SystemTime;

use super::{
    BreezSdk, SyncRequest, SyncType,
    helpers::{InternalEventListener, is_payment_match},
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
            ReceivePaymentMethod::BitcoinAddress => {
                let object_repository = ObjectCacheRepository::new(self.storage.clone());

                // First lookup in storage cache
                let static_deposit_address =
                    object_repository.fetch_static_deposit_address().await?;
                if let Some(static_deposit_address) = static_deposit_address {
                    return Ok(ReceivePaymentResponse {
                        payment_request: static_deposit_address.address.clone(),
                        fee: 0,
                    });
                }

                // Then query existing addresses
                let deposit_addresses = self
                    .spark_wallet
                    .list_static_deposit_addresses(None)
                    .await?;

                // In case there are no addresses, generate a new one and cache it
                let address = match deposit_addresses.items.last() {
                    Some(address) => address.to_string(),
                    None => self
                        .spark_wallet
                        .generate_deposit_address(true)
                        .await?
                        .to_string(),
                };

                object_repository
                    .save_static_deposit_address(&StaticDepositAddress {
                        address: address.clone(),
                    })
                    .await?;

                Ok(ReceivePaymentResponse {
                    payment_request: address,
                    fee: 0,
                })
            }
            ReceivePaymentMethod::Bolt11Invoice {
                description,
                amount_sats,
                expiry_secs,
            } => Ok(ReceivePaymentResponse {
                payment_request: self
                    .spark_wallet
                    .create_lightning_invoice(
                        amount_sats.unwrap_or_default(),
                        Some(InvoiceDescription::Memo(description.clone())),
                        None,
                        expiry_secs,
                        self.config.prefer_spark_over_lightning,
                    )
                    .await?
                    .invoice,
                fee: 0,
            }),
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
        let parsed_input = self.parse(&request.payment_request).await?;

        validate_prepare_send_payment_request(
            &parsed_input,
            &request,
            &self.spark_wallet.get_identity_public_key().to_string(),
        )?;

        // Extract amount and token_identifier from pay_amount
        let (request_amount, token_identifier): (Option<u128>, Option<String>) =
            match &request.pay_amount {
                Some(PayAmount::Bitcoin { amount_sats }) => (Some(u128::from(*amount_sats)), None),
                Some(PayAmount::Token {
                    amount,
                    token_identifier,
                }) => (Some(*amount), Some(token_identifier.clone())),
                Some(PayAmount::Drain) | None => (None, None), // Drain handled separately
            };

        match &parsed_input {
            InputType::SparkAddress(spark_address_details) => {
                let is_drain = matches!(request.pay_amount, Some(PayAmount::Drain));

                let (pay_amount, conversion_estimate) = if is_drain {
                    // Drain doesn't support conversion (validated earlier)
                    (PayAmount::Drain, None)
                } else {
                    let amount = request_amount
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                    let conversion_estimate = self
                        .token_converter
                        .validate(
                            request.conversion_options.as_ref(),
                            token_identifier.as_ref(),
                            amount,
                        )
                        .await?;
                    let pay_amount =
                        PayAmount::from_amount_and_token(amount, token_identifier.clone())?;
                    (pay_amount, conversion_estimate)
                };

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkAddress {
                        address: spark_address_details.address.clone(),
                        fee: 0,
                        token_identifier,
                    },
                    pay_amount,
                    conversion_estimate,
                })
            }
            InputType::SparkInvoice(spark_invoice_details) => {
                let is_drain = matches!(request.pay_amount, Some(PayAmount::Drain));

                // Use request's token_identifier if provided, otherwise fall back to invoice's
                let effective_token_identifier = token_identifier
                    .clone()
                    .or_else(|| spark_invoice_details.token_identifier.clone());

                let (pay_amount, conversion_estimate) = if is_drain {
                    // Drain only allowed for amountless invoices (validated earlier)
                    // Drain doesn't support conversion (validated earlier)
                    (PayAmount::Drain, None)
                } else {
                    let amount = spark_invoice_details
                        .amount
                        .or(request_amount)
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                    let conversion_estimate = self
                        .token_converter
                        .validate(
                            request.conversion_options.as_ref(),
                            effective_token_identifier.as_ref(),
                            amount,
                        )
                        .await?;
                    let pay_amount = PayAmount::from_amount_and_token(
                        amount,
                        effective_token_identifier.clone(),
                    )?;
                    (pay_amount, conversion_estimate)
                };

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::SparkInvoice {
                        spark_invoice_details: spark_invoice_details.clone(),
                        fee: 0,
                        token_identifier: effective_token_identifier,
                    },
                    pay_amount,
                    conversion_estimate,
                })
            }
            InputType::Bolt11Invoice(detailed_bolt11_invoice) => {
                let spark_address: Option<SparkAddress> = self
                    .spark_wallet
                    .extract_spark_address(&request.payment_request)?;

                let spark_transfer_fee_sats = if spark_address.is_some() {
                    Some(0)
                } else {
                    None
                };

                let amount = request_amount
                    .or(detailed_bolt11_invoice
                        .amount_msat
                        .map(|msat| u128::from(msat).saturating_div(1000)))
                    .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                let lightning_fee_sats = self
                    .spark_wallet
                    .fetch_lightning_send_fee_estimate(
                        &request.payment_request,
                        request_amount
                            .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                            .transpose()?,
                    )
                    .await?;
                let conversion_estimate = self
                    .token_converter
                    .validate(
                        request.conversion_options.as_ref(),
                        token_identifier.as_ref(),
                        amount.saturating_add(u128::from(lightning_fee_sats)),
                    )
                    .await?;

                let pay_amount = PayAmount::from_amount_and_token(amount, token_identifier)?;

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::Bolt11Invoice {
                        invoice_details: detailed_bolt11_invoice.clone(),
                        spark_transfer_fee_sats,
                        lightning_fee_sats,
                    },
                    pay_amount,
                    conversion_estimate,
                })
            }
            InputType::BitcoinAddress(withdrawal_address) => {
                let is_drain = matches!(request.pay_amount, Some(PayAmount::Drain));

                // For drain, pass None to get drain-specific fees; otherwise pass the amount
                let amount_for_quote = (!is_drain)
                    .then(|| {
                        request_amount
                            .ok_or(SdkError::InvalidInput("Amount is required".to_string()))
                            .and_then(|a| a.try_into().map_err(Into::into))
                    })
                    .transpose()?;

                let fee_quote: SendOnchainFeeQuote = self
                    .spark_wallet
                    .fetch_coop_exit_fee_quote(&withdrawal_address.address, amount_for_quote)
                    .await?
                    .into();

                let (pay_amount, conversion_estimate) = if is_drain {
                    // Drain doesn't support conversion (validated earlier)
                    (PayAmount::Drain, None)
                } else {
                    let amount: u64 = amount_for_quote
                        .ok_or(SdkError::InvalidInput("Amount is required".to_string()))?;
                    // For conversion estimate, use fast fee as worst case
                    let fee_sats_for_estimate = fee_quote.speed_fast.total_fee_sat();
                    let conversion_estimate = self
                        .token_converter
                        .validate(
                            request.conversion_options.as_ref(),
                            token_identifier.as_ref(),
                            u128::from(amount).saturating_add(u128::from(fee_sats_for_estimate)),
                        )
                        .await?;
                    (
                        PayAmount::Bitcoin {
                            amount_sats: amount,
                        },
                        conversion_estimate,
                    )
                };

                Ok(PrepareSendPaymentResponse {
                    payment_method: SendPaymentMethod::BitcoinAddress {
                        address: withdrawal_address.clone(),
                        fee_quote,
                    },
                    pay_amount,
                    conversion_estimate,
                })
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
        let mut payments = self.storage.list_payments(request).await?;

        // Collect all parent IDs and batch query for related payments
        let parent_ids: Vec<String> = payments.iter().map(|p| p.id.clone()).collect();

        if !parent_ids.is_empty() {
            let related_payments_map = self.storage.get_payments_by_parent_ids(parent_ids).await?;

            // Add conversion details of each payments
            for payment in &mut payments {
                if let Some(related_payments) = related_payments_map.get(&payment.id) {
                    match related_payments.try_into() {
                        Ok(conversion_details) => {
                            payment.conversion_details = Some(conversion_details);
                        }
                        Err(e) => {
                            warn!("Found payments couldn't be converted to ConversionDetails: {e}");
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
        let mut payment = self.storage.get_payment_by_id(request.payment_id).await?;

        // Load related payments (single ID batch)
        let related_payments_map = self
            .storage
            .get_payments_by_parent_ids(vec![payment.id.clone()])
            .await?;

        if let Some(related_payments) = related_payments_map.get(&payment.id) {
            match related_payments.try_into() {
                Ok(conversion_details) => payment.conversion_details = Some(conversion_details),
                Err(e) => {
                    warn!("Related payments not convertable to ConversionDetails: {e}");
                }
            }
        }

        Ok(GetPaymentResponse { payment })
    }
}

// Private payment methods
impl BreezSdk {
    pub(super) async fn maybe_convert_token_send_payment(
        &self,
        request: SendPaymentRequest,
        mut suppress_payment_event: bool,
        amount_override: Option<u64>,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Extract token_identifier from pay_amount
        let token_identifier = match &request.prepare_response.pay_amount {
            PayAmount::Token {
                token_identifier, ..
            } => Some(token_identifier.clone()),
            _ => None,
        };

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
        // Perform the send payment, with conversion if requested
        let res = if let Some(ConversionEstimate {
            options: conversion_options,
            ..
        }) = &request.prepare_response.conversion_estimate
        {
            Box::pin(self.convert_token_send_payment_internal(
                conversion_options,
                &request,
                &mut suppress_payment_event,
            ))
            .await
        } else {
            Box::pin(self.send_payment_internal(&request, amount_override)).await
        };
        // Emit payment status event and trigger wallet state sync
        if let Ok(response) = &res {
            if !suppress_payment_event {
                self.event_emitter
                    .emit(&SdkEvent::from_payment(response.payment.clone()))
                    .await;
            }
            if let Err(e) = self
                .sync_trigger
                .send(SyncRequest::no_reply(SyncType::WalletState))
            {
                error!("Failed to send sync trigger: {e:?}");
            }
        }
        res
    }

    #[allow(clippy::too_many_lines)]
    async fn convert_token_send_payment_internal(
        &self,
        conversion_options: &ConversionOptions,
        request: &SendPaymentRequest,
        suppress_payment_event: &mut bool,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Extract amount and token_identifier from pay_amount
        let (amount, token_identifier) = match &request.prepare_response.pay_amount {
            PayAmount::Bitcoin { amount_sats } => (u128::from(*amount_sats), None),
            PayAmount::Token {
                amount,
                token_identifier,
            } => (*amount, Some(token_identifier.clone())),
            PayAmount::Drain => {
                return Err(SdkError::InvalidInput(
                    "Drain not supported with token conversion".to_string(),
                ));
            }
        };

        // Perform a conversion before sending the payment
        let (conversion_response, conversion_purpose) =
            match &request.prepare_response.payment_method {
                SendPaymentMethod::SparkAddress { address, .. } => {
                    let spark_address = address
                        .parse::<SparkAddress>()
                        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
                    let conversion_purpose = if spark_address.identity_public_key
                        == self.spark_wallet.get_identity_public_key()
                    {
                        ConversionPurpose::SelfTransfer
                    } else {
                        ConversionPurpose::OngoingPayment {
                            payment_request: address.clone(),
                        }
                    };
                    let res = self
                        .token_converter
                        .convert(
                            conversion_options,
                            &conversion_purpose,
                            token_identifier.as_ref(),
                            amount,
                        )
                        .await?;
                    (res, conversion_purpose)
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
                    let conversion_purpose = if identity_public_key == &own_identity_public_key {
                        ConversionPurpose::SelfTransfer
                    } else {
                        ConversionPurpose::OngoingPayment {
                            payment_request: invoice.clone(),
                        }
                    };
                    let res = self
                        .token_converter
                        .convert(
                            conversion_options,
                            &conversion_purpose,
                            token_identifier.as_ref(),
                            amount,
                        )
                        .await?;
                    (res, conversion_purpose)
                }
                SendPaymentMethod::Bolt11Invoice {
                    spark_transfer_fee_sats,
                    lightning_fee_sats,
                    invoice_details,
                    ..
                } => {
                    let conversion_purpose = ConversionPurpose::OngoingPayment {
                        payment_request: invoice_details.invoice.bolt11.clone(),
                    };
                    let res = self
                        .convert_token_for_bolt11_invoice(
                            conversion_options,
                            *spark_transfer_fee_sats,
                            *lightning_fee_sats,
                            request,
                            &conversion_purpose,
                            amount,
                            token_identifier.as_ref(),
                        )
                        .await?;
                    (res, conversion_purpose)
                }
                SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
                    let conversion_purpose = ConversionPurpose::OngoingPayment {
                        payment_request: address.address.clone(),
                    };
                    let res = self
                        .convert_token_for_bitcoin_address(
                            conversion_options,
                            fee_quote,
                            request,
                            &conversion_purpose,
                            amount,
                            token_identifier.as_ref(),
                        )
                        .await?;
                    (res, conversion_purpose)
                }
            };
        // Trigger a wallet state sync if converting from Bitcoin to token
        if matches!(
            conversion_options.conversion_type,
            ConversionType::FromBitcoin
        ) {
            let _ = self
                .sync_trigger
                .send(SyncRequest::no_reply(SyncType::WalletState));
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
        // For self-payments, we can skip sending the actual payment
        if conversion_purpose == ConversionPurpose::SelfTransfer {
            *suppress_payment_event = true;
            return Ok(SendPaymentResponse { payment });
        }
        // Now send the actual payment
        let response = Box::pin(self.send_payment_internal(request, None)).await?;
        // Merge payment metadata to link the payments
        self.merge_payment_metadata(
            conversion_response.sent_payment_id,
            PaymentMetadata {
                parent_payment_id: Some(response.payment.id.clone()),
                ..Default::default()
            },
        )
        .await?;
        self.merge_payment_metadata(
            conversion_response.received_payment_id,
            PaymentMetadata {
                parent_payment_id: Some(response.payment.id.clone()),
                ..Default::default()
            },
        )
        .await?;
        // Fetch the updated payment with conversion details
        self.get_payment(GetPaymentRequest {
            payment_id: response.payment.id,
        })
        .await
        .map(|res| SendPaymentResponse {
            payment: res.payment,
        })
    }

    pub(super) async fn send_payment_internal(
        &self,
        request: &SendPaymentRequest,
        amount_override: Option<u64>,
    ) -> Result<SendPaymentResponse, SdkError> {
        // Extract the amount from pay_amount
        let amount = match &request.prepare_response.pay_amount {
            PayAmount::Bitcoin { amount_sats } => u128::from(*amount_sats),
            PayAmount::Token { amount, .. } => *amount,
            PayAmount::Drain => {
                // For drain, amount is computed at send time in send_bitcoin_address
                0
            }
        };

        match &request.prepare_response.payment_method {
            SendPaymentMethod::SparkAddress {
                address,
                token_identifier,
                ..
            } => {
                // For drain, get fresh balance at send time
                let send_amount = if matches!(request.prepare_response.pay_amount, PayAmount::Drain)
                {
                    u128::from(self.spark_wallet.get_balance().await?)
                } else {
                    amount
                };
                self.send_spark_address(
                    address,
                    token_identifier.clone(),
                    send_amount,
                    request.options.as_ref(),
                    request.idempotency_key.clone(),
                )
                .await
            }
            SendPaymentMethod::SparkInvoice {
                spark_invoice_details,
                ..
            } => {
                // For drain, get fresh balance at send time
                let send_amount = if matches!(request.prepare_response.pay_amount, PayAmount::Drain)
                {
                    u128::from(self.spark_wallet.get_balance().await?)
                } else {
                    amount
                };
                self.send_spark_invoice(&spark_invoice_details.invoice, request, send_amount)
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
                self.send_bitcoin_address(address, fee_quote, request).await
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

    async fn send_bolt11_invoice(
        &self,
        invoice_details: &Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
        request: &SendPaymentRequest,
        amount_override: Option<u64>,
        amount: u128,
    ) -> Result<SendPaymentResponse, SdkError> {
        let amount_to_send = match amount_override {
            // Amount override provided (e.g., for drain overpayment)
            Some(amt) => Some(amt.into()),
            None => match invoice_details.amount_msat {
                // We are not sending amount in case the invoice contains it.
                Some(_) => None,
                // We are sending amount for zero amount invoice
                None => Some(amount),
            },
        };
        let (prefer_spark, completion_timeout_secs) = match request.options {
            Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark,
                completion_timeout_secs,
            }) => (prefer_spark, completion_timeout_secs),
            _ => (self.config.prefer_spark_over_lightning, None),
        };
        let fee_sats = match (prefer_spark, spark_transfer_fee_sats, lightning_fee_sats) {
            (true, Some(fee), _) => fee,
            _ => lightning_fee_sats,
        };
        let transfer_id = request
            .idempotency_key
            .as_ref()
            .map(|idempotency_key| TransferId::from_str(idempotency_key))
            .transpose()?;

        let payment_response = self
            .spark_wallet
            .pay_lightning_invoice(
                &invoice_details.invoice.bolt11,
                amount_to_send
                    .map(|a| Ok::<u64, SdkError>(a.try_into()?))
                    .transpose()?,
                Some(fee_sats),
                prefer_spark,
                transfer_id,
            )
            .await?;
        let payment = match payment_response.lightning_payment {
            Some(lightning_payment) => {
                let ssp_id = lightning_payment.id.clone();
                let payment = Payment::from_lightning(
                    lightning_payment,
                    amount,
                    payment_response.transfer.id.to_string(),
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

        // Compute amount - for drain, calculate at send time
        let amount_sats: u64 = match &request.prepare_response.pay_amount {
            PayAmount::Bitcoin { amount_sats } => *amount_sats,
            PayAmount::Drain => {
                let balance_sats = self.spark_wallet.get_balance().await?;
                balance_sats.saturating_sub(fee_sats)
            }
            PayAmount::Token { .. } => {
                return Err(SdkError::InvalidInput(
                    "Token payments not supported for Bitcoin address".to_string(),
                ));
            }
        };

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
        let id = self
            .add_event_listener(Box::new(InternalEventListener::new(tx)))
            .await;

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
            self.remove_event_listener(&id).await;
            return Ok(payment);
        }

        let timeout_res = timeout(Duration::from_secs(completion_timeout_secs.into()), async {
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
        .map_err(|_| SdkError::Generic("Timeout waiting for payment".to_string()));

        self.remove_event_listener(&id).await;
        timeout_res?
    }

    async fn merge_payment_metadata(
        &self,
        payment_id: String,
        mut metadata: PaymentMetadata,
    ) -> Result<(), SdkError> {
        if let Some(details) = self
            .storage
            .get_payment_by_id(payment_id.clone())
            .await
            .ok()
            .and_then(|p| p.details)
        {
            match details {
                PaymentDetails::Lightning {
                    lnurl_pay_info,
                    lnurl_withdraw_info,
                    ..
                } => {
                    metadata.lnurl_pay_info = metadata.lnurl_pay_info.or(lnurl_pay_info);
                    metadata.lnurl_withdraw_info =
                        metadata.lnurl_withdraw_info.or(lnurl_withdraw_info);
                }
                PaymentDetails::Spark {
                    conversion_info, ..
                }
                | PaymentDetails::Token {
                    conversion_info, ..
                } => {
                    metadata.conversion_info = metadata.conversion_info.or(conversion_info);
                }
                _ => {}
            }
        }
        self.storage
            .set_payment_metadata(payment_id, metadata)
            .await?;
        Ok(())
    }

    // Pools the lightning send payment untill it is in completed state.
    fn poll_lightning_send_payment(&self, payment: &Payment, ssp_id: String) {
        const MAX_POLL_ATTEMPTS: u32 = 20;
        let payment_id = payment.id.clone();
        info!("Polling lightning send payment {}", payment_id);

        let spark_wallet = self.spark_wallet.clone();
        let sync_trigger = self.sync_trigger.clone();
        let event_emitter = self.event_emitter.clone();
        let payment = payment.clone();
        let payment_id = payment_id.clone();
        let mut shutdown = self.shutdown_sender.subscribe();

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
                        if let Ok(Some(p)) = p && let Ok(payment) = Payment::from_lightning(p.clone(), payment.amount, payment.id.clone()) {
                            info!("Polling payment status = {} {:?}", payment.status, p.status);
                            if payment.status != PaymentStatus::Pending {
                                info!("Polling payment completed status = {}", payment.status);
                                event_emitter.emit(&SdkEvent::from_payment(payment.clone())).await;
                                if let Err(e) = sync_trigger.send(SyncRequest::no_reply(SyncType::WalletState)) {
                                    error!("Failed to send sync trigger: {e:?}");
                                }
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
        });
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
    ) -> Result<TokenConversionResponse, SdkError> {
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

        self.token_converter
            .convert(
                conversion_options,
                conversion_purpose,
                token_identifier,
                min_amount_out,
            )
            .await
            .map_err(Into::into)
    }

    async fn convert_token_for_bitcoin_address(
        &self,
        conversion_options: &ConversionOptions,
        fee_quote: &SendOnchainFeeQuote,
        request: &SendPaymentRequest,
        conversion_purpose: &ConversionPurpose,
        amount: u128,
        token_identifier: Option<&String>,
    ) -> Result<TokenConversionResponse, SdkError> {
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

        self.token_converter
            .convert(
                conversion_options,
                conversion_purpose,
                token_identifier,
                min_amount_out,
            )
            .await
            .map_err(Into::into)
    }
}
