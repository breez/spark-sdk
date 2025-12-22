use std::sync::{Arc, Weak};

use breez_sdk_spark::{BreezSdk, GetInfoRequest, ListPaymentsResponse, PrepareSendPaymentRequest};
use breez_sdk_spark::{ListPaymentsRequest, PaymentDetails, PaymentType, SendPaymentRequest};
use maybe_sync::{MaybeSend, MaybeSync};
use nostr_sdk::nips::nip47::{
    ErrorCode, GetBalanceResponse, ListTransactionsRequest, LookupInvoiceResponse, NIP47Error,
    PayInvoiceRequest, PayInvoiceResponse, TransactionType,
};
use nostr_sdk::Timestamp;
use tracing::info;

type Result<T> = std::result::Result<T, NIP47Error>;

#[macros::async_trait]
pub trait RelayMessageHandler: MaybeSend + MaybeSync {
    async fn pay_invoice(&self, req: PayInvoiceRequest) -> Result<PayInvoiceResponse>;
    async fn list_transactions(
        &self,
        req: ListTransactionsRequest,
    ) -> Result<Vec<LookupInvoiceResponse>>;
    async fn get_balance(&self) -> Result<GetBalanceResponse>;
}

pub struct SdkRelayMessageHandler {
    sdk: Weak<BreezSdk>,
}

impl SdkRelayMessageHandler {
    pub fn new(sdk: Weak<BreezSdk>) -> Self {
        Self { sdk }
    }

    fn get_sdk(&self) -> Result<Arc<BreezSdk>> {
        let Some(sdk) = self.sdk.upgrade() else {
            return Err(NIP47Error {
                code: ErrorCode::Internal,
                message: "Could not handle message: SDK is not running.".to_string(),
            });
        };
        Ok(sdk)
    }
}

#[macros::async_trait]
impl RelayMessageHandler for SdkRelayMessageHandler {
    /// Processes a Lightning invoice payment request.
    ///
    /// This method handles the complete payment flow:
    /// 1. Prepares the payment using the SDK
    /// 2. Executes the payment
    /// 3. Extracts the preimage and fees from the completed payment
    ///
    /// # Arguments
    /// * `req` - Payment request containing invoice and optional amount override
    ///
    /// # Returns
    /// * `Ok(PayInvoiceResponse)` - Contains payment preimage and fees paid
    /// * `Err(NIP47Error)` - Payment preparation or execution error
    async fn pay_invoice(&self, req: PayInvoiceRequest) -> Result<PayInvoiceResponse> {
        // Create prepare request
        info!("NWC Pay invoice is called");
        let sdk = self.get_sdk()?;

        let prepare_req = PrepareSendPaymentRequest {
            payment_request: req.invoice,
            amount: req.amount.map(|a| (a / 1000) as u128),
            token_identifier: None,
            token_conversion_options: None,
        };

        // Prepare the payment
        let prepare_resp = sdk
            .prepare_send_payment(prepare_req)
            .await
            .map_err(|e| NIP47Error {
                code: ErrorCode::PaymentFailed,
                message: format!("Failed to prepare payment: {e}"),
            })?;

        // Create send request
        let send_req = SendPaymentRequest {
            prepare_response: prepare_resp,
            options: None,
            idempotency_key: None,
        };

        // Send the payment
        let response = sdk.send_payment(send_req).await.map_err(|e| NIP47Error {
            code: ErrorCode::PaymentFailed,
            message: format!("Failed to send payment: {e}"),
        })?;

        // Extract preimage and fees from payment
        let Some(PaymentDetails::Lightning {
            preimage: Some(preimage),
            ..
        }) = response.payment.details
        else {
            return Err(NIP47Error {
                code: ErrorCode::PaymentFailed,
                message: "Payment did not return any preimage".to_string(),
            });
        };

        let fees_paid = (response.payment.fees * 1000) as u64; // Convert sats to msats

        Ok(PayInvoiceResponse {
            preimage,
            fees_paid: Some(fees_paid),
        })
    }

    /// Retrieves a filtered list of wallet transactions.
    ///
    /// This method converts NIP-47 transaction filters to Breez payment filters
    /// and returns transactions in the expected NIP-47 format.
    ///
    /// # Arguments
    /// * `req` - Filter criteria including transaction type, unpaid status, time range, and pagination
    ///
    /// # Returns
    /// * `Ok(Vec<LookupInvoiceResponse>)` - List of transactions matching the filters
    /// * `Err(NIP47Error)` - Error retrieving payments from the SDK
    async fn list_transactions(
        &self,
        req: ListTransactionsRequest,
    ) -> Result<Vec<LookupInvoiceResponse>> {
        // TODO: Add filters
        // let filters = req.transaction_type.map(|p| {
        //     vec![match p {
        //         TransactionType::Incoming => PaymentType::Receive,
        //         TransactionType::Outgoing => PaymentType::Send,
        //     }]
        // });
        // let states = req.unpaid.and_then(|unpaid| {
        //     if unpaid {
        //         None
        //     } else {
        //         Some(vec![PaymentStatus::Completed])
        //     }
        // });
        info!("NWC List transactions is called");

        // Get payments from SDK
        let ListPaymentsResponse { payments } = self
            .get_sdk()?
            .list_payments(ListPaymentsRequest {
                type_filter: None,
                status_filter: None,
                asset_filter: None,
                payment_details_filter: None,
                from_timestamp: None,
                to_timestamp: None,
                limit: req.limit.map(|l| l as u32),
                offset: req.offset.map(|o| o as u32),
                sort_ascending: None,
            })
            .await
            .map_err(|e| NIP47Error {
                code: ErrorCode::Internal,
                message: format!("Failed to list payments: {e}"),
            })?;

        // Convert payments to NIP-47 transactions
        let txs: Vec<LookupInvoiceResponse> = payments
            .into_iter()
            .map(|payment| {
                let (description, preimage, invoice, payment_hash) = match payment.details {
                    Some(PaymentDetails::Lightning {
                        description,
                        preimage,
                        invoice,
                        payment_hash,
                        ..
                    }) => (description, preimage, Some(invoice), Some(payment_hash)),
                    _ => (None, None, None, None),
                };

                LookupInvoiceResponse {
                    payment_hash: payment_hash.unwrap_or_else(|| "null".to_string()),
                    transaction_type: Some(match payment.payment_type {
                        PaymentType::Receive => TransactionType::Incoming,
                        PaymentType::Send => TransactionType::Outgoing,
                    }),
                    invoice,
                    description,
                    preimage,
                    amount: (payment.amount * 1000) as u64,
                    fees_paid: (payment.fees * 1000) as u64,
                    created_at: Timestamp::from_secs(payment.timestamp),
                    description_hash: None,
                    expires_at: None,
                    settled_at: None,
                    metadata: None,
                }
            })
            .collect();

        Ok(txs)
    }

    /// Retrieves the current wallet balance.
    ///
    /// # Returns
    /// * `Ok(GetBalanceResponse)` - Balance in millisatoshis
    /// * `Err(NIP47Error)` - Error getting wallet info from the SDK
    async fn get_balance(&self) -> Result<GetBalanceResponse> {
        info!("NWC Get balance is called");
        let info = self
            .get_sdk()?
            .get_info(GetInfoRequest { ensure_synced: None })
            .await
            .map_err(|e| NIP47Error {
                code: ErrorCode::Internal,
                message: format!("Failed to get wallet info: {e}"),
            })?;

        let balance_msats = info.balance_sats * 1000;

        Ok(GetBalanceResponse {
            balance: balance_msats,
        })
    }
}
