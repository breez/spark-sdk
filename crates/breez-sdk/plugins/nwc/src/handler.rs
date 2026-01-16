use std::sync::Arc;

use breez_sdk_spark::{
    GetInfoRequest, ListPaymentsRequest, ListPaymentsResponse, PaymentDetails, PaymentType,
    SdkServices,
};
use nostr_sdk::nips::nip47::{
    ErrorCode, GetBalanceResponse, ListTransactionsRequest, LookupInvoiceResponse, NIP47Error,
    PayInvoiceRequest, PayInvoiceResponse, TransactionType,
};
use nostr_sdk::Timestamp;
use tracing::info;

type Result<T> = std::result::Result<T, NIP47Error>;

#[macros::async_trait]
pub trait RelayMessageHandler: Send + Sync {
    async fn pay_invoice(&self, req: PayInvoiceRequest) -> Result<PayInvoiceResponse>;
    async fn list_transactions(
        &self,
        req: ListTransactionsRequest,
    ) -> Result<Vec<LookupInvoiceResponse>>;
    async fn get_balance(&self) -> Result<GetBalanceResponse>;
}

pub struct SdkRelayMessageHandler {
    services: Arc<SdkServices>,
}

impl SdkRelayMessageHandler {
    pub fn new(services: Arc<SdkServices>) -> Self {
        Self { services }
    }
}

#[macros::async_trait]
impl RelayMessageHandler for SdkRelayMessageHandler {
    /// Processes a Lightning invoice payment request.
    ///
    /// This method handles the complete payment flow using the SparkWallet directly:
    /// 1. Pays the lightning invoice
    /// 2. Extracts the preimage and fees from the completed payment
    ///
    /// # Arguments
    /// * `req` - Payment request containing invoice and optional amount override
    ///
    /// # Returns
    /// * `Ok(PayInvoiceResponse)` - Contains payment preimage and fees paid
    /// * `Err(NIP47Error)` - Payment preparation or execution error
    async fn pay_invoice(&self, req: PayInvoiceRequest) -> Result<PayInvoiceResponse> {
        info!("NWC Pay invoice is called");

        // Pay the lightning invoice using SdkServices
        let result = self
            .services
            .pay_lightning_invoice(
                &req.invoice,
                req.amount.map(|a| (a / 1000) as u64), // Convert msats to sats
            )
            .await
            .map_err(|e| NIP47Error {
                code: ErrorCode::PaymentFailed,
                message: format!("Failed to pay invoice: {e}"),
            })?;

        // Get the lightning payment from the result
        let lightning_payment = result.lightning_payment.ok_or_else(|| NIP47Error {
            code: ErrorCode::PaymentFailed,
            message: "Payment did not return lightning payment details".to_string(),
        })?;

        // Get preimage from payment
        let preimage = lightning_payment
            .payment_preimage
            .ok_or_else(|| NIP47Error {
                code: ErrorCode::PaymentFailed,
                message: "Payment did not return any preimage".to_string(),
            })?;

        let fees_paid = lightning_payment.fee_sat * 1000; // Convert sats to msats

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
        info!("NWC List transactions is called");

        // Get payments using SdkServices
        let ListPaymentsResponse { payments } = self
            .services
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
            .services
            .get_info(GetInfoRequest {
                ensure_synced: None,
            })
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
