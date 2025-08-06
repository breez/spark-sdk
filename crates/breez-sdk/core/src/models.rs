use breez_sdk_common::input::{self, BitcoinAddress, DetailedBolt11Invoice};
use core::fmt;
use serde::{Deserialize, Serialize};
use spark_wallet::{
    CurrencyAmount, LightningSendPayment, LightningSendStatus, Network as SparkNetwork,
    SspUserRequest, TransferDirection, TransferStatus, WalletTransfer,
};
use std::time::UNIX_EPOCH;

use crate::SdkError;

/// The type of payment
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PaymentType {
    /// Payment sent from this wallet
    Send,
    /// Payment received to this wallet
    Receive,
}

impl fmt::Display for PaymentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentType::Send => write!(f, "send"),
            PaymentType::Receive => write!(f, "receive"),
        }
    }
}

impl From<&str> for PaymentType {
    fn from(s: &str) -> Self {
        match s {
            "receive" => PaymentType::Receive,
            _ => PaymentType::Send, // Default to Send if unknown or 'send'
        }
    }
}

/// The status of a payment
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PaymentStatus {
    /// Payment is completed successfully
    Completed,
    /// Payment is in progress
    Pending,
    /// Payment has failed
    Failed,
}

impl fmt::Display for PaymentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentStatus::Completed => write!(f, "completed"),
            PaymentStatus::Pending => write!(f, "pending"),
            PaymentStatus::Failed => write!(f, "failed"),
        }
    }
}

impl From<&str> for PaymentStatus {
    fn from(s: &str) -> Self {
        match s {
            "completed" => PaymentStatus::Completed,
            "failed" => PaymentStatus::Failed,
            _ => PaymentStatus::Pending, // Default to Pending if unknown or 'pending'
        }
    }
}

/// Represents a payment (sent or received)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    /// Unique identifier for the payment
    pub id: String,
    /// Type of payment (send or receive)
    pub payment_type: PaymentType,
    /// Status of the payment
    pub status: PaymentStatus,
    /// Amount in satoshis
    pub amount: u64,
    /// Fee paid in satoshis
    pub fees: u64,
    /// Timestamp of when the payment was created
    pub timestamp: u64,
    /// Details of the payment
    pub details: PaymentDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaymentDetails {
    Spark,
    Lightning {
        /// Represents the invoice description
        description: Option<String>,
        /// The preimage of the paid invoice (proof of payment).
        preimage: Option<String>,
        /// Represents the Bolt11/Bolt12 invoice associated with a payment
        /// In the case of a Send payment, this is the invoice paid to the user
        /// In the case of a Receive payment, this is the invoice paid by the user
        invoice: String,

        /// The payment hash of the invoice
        payment_hash: String,

        /// The invoice destination/payee pubkey
        destination_pubkey: String,
    },
    Withdraw {
        tx_id: String,
    },
    Deposit {
        tx_id: String,
    },
}

impl TryFrom<SspUserRequest> for PaymentDetails {
    type Error = SdkError;
    fn try_from(user_request: SspUserRequest) -> Result<Self, Self::Error> {
        let details = match user_request {
            SspUserRequest::CoopExitRequest(request) => PaymentDetails::Withdraw {
                tx_id: request.coop_exit_txid,
            },
            SspUserRequest::LeavesSwapRequest(_) => PaymentDetails::Spark,
            SspUserRequest::LightningReceiveRequest(request) => {
                let detailed_invoice = input::parse_invoice(&request.invoice.encoded_invoice)
                    .ok_or(SdkError::GenericError(
                        "Invalid invoice in SspUserRequest::LightningReceiveRequest".to_string(),
                    ))?;
                PaymentDetails::Lightning {
                    description: request.invoice.memo,
                    preimage: request.lightning_receive_payment_preimage,
                    invoice: request.invoice.encoded_invoice,
                    payment_hash: request.invoice.payment_hash,
                    destination_pubkey: detailed_invoice.payee_pubkey,
                }
            }
            SspUserRequest::LightningSendRequest(request) => {
                let detailed_invoice = input::parse_invoice(&request.encoded_invoice).ok_or(
                    SdkError::GenericError(
                        "Invalid invoice in SspUserRequest::LightningSendRequest".to_string(),
                    ),
                )?;
                PaymentDetails::Lightning {
                    description: detailed_invoice.description,
                    preimage: request.lightning_send_payment_preimage,
                    invoice: request.encoded_invoice,
                    payment_hash: detailed_invoice.payment_hash,
                    destination_pubkey: detailed_invoice.payee_pubkey,
                }
            }
            SspUserRequest::ClaimStaticDeposit(request) => PaymentDetails::Deposit {
                tx_id: request.transaction_id,
            },
        };
        Ok(details)
    }
}

impl TryFrom<WalletTransfer> for Payment {
    type Error = SdkError;
    fn try_from(transfer: WalletTransfer) -> Result<Self, Self::Error> {
        let payment_type = match transfer.direction {
            TransferDirection::Incoming => PaymentType::Receive,
            TransferDirection::Outgoing => PaymentType::Send,
        };
        let status = match transfer.status {
            TransferStatus::Completed => PaymentStatus::Completed,
            TransferStatus::Expired => PaymentStatus::Failed,
            TransferStatus::Returned => PaymentStatus::Failed,
            _ => PaymentStatus::Pending,
        };
        let fees: CurrencyAmount = match transfer.clone().user_request {
            Some(user_request) => match user_request {
                SspUserRequest::LightningSendRequest(r) => r.fee,
                SspUserRequest::CoopExitRequest(r) => r.fee,
                _ => CurrencyAmount::default(),
            },
            None => CurrencyAmount::default(),
        };

        let details: PaymentDetails = match transfer.user_request {
            Some(user_request) => user_request.try_into()?,
            None => PaymentDetails::Spark,
        };

        Ok(Payment {
            id: transfer.id.to_string(),
            payment_type,
            status,
            amount: transfer.total_value_sat,
            fees: fees.as_sats().unwrap_or(0),
            timestamp: match transfer.created_at.map(|t| t.duration_since(UNIX_EPOCH)) {
                Some(Ok(duration)) => duration.as_secs(),
                _ => 0,
            },
            details,
        })
    }
}

impl Payment {
    pub fn from_lightning(
        payment: LightningSendPayment,
        amount_sat: u64,
    ) -> Result<Self, SdkError> {
        let status = match payment.status {
            LightningSendStatus::LightningPaymentSucceeded => PaymentStatus::Completed,
            LightningSendStatus::LightningPaymentFailed => PaymentStatus::Failed,
            _ => PaymentStatus::Pending,
        };

        let detailed_invoice = input::parse_invoice(&payment.encoded_invoice).ok_or(
            SdkError::GenericError("Invalid invoice in LightnintSendPayment".to_string()),
        )?;
        let details = PaymentDetails::Lightning {
            description: detailed_invoice.description,
            preimage: payment.payment_preimage,
            invoice: payment.encoded_invoice,
            payment_hash: detailed_invoice.payment_hash,
            destination_pubkey: detailed_invoice.payee_pubkey,
        };

        Ok(Payment {
            id: payment.id,
            payment_type: PaymentType::Send,
            status,
            amount: amount_sat,
            fees: payment.fee_sat,
            timestamp: payment.created_at as u64,
            details,
        })
    }
}

#[derive(Debug, Clone)]
pub enum Network {
    Mainnet,
    Regtest,
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Network::Mainnet => write!(f, "Mainnet"),
            Network::Regtest => write!(f, "Regtest"),
        }
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

#[derive(Debug, Clone)]
pub struct Config {
    pub network: Network,
    pub mnemonic: String,
    pub data_dir: String,
}

/// Request to get the balance of the wallet
#[derive(Debug, Clone)]
pub struct GetInfoRequest {}

/// Response containing the balance of the wallet
#[derive(Debug, Clone, Serialize)]
pub struct GetInfoResponse {
    /// The balance in satoshis
    pub balance_sats: u64,
}

/// Request to sync the wallet with the Spark network
#[derive(Debug, Clone)]
pub struct SyncWalletRequest {}

/// Response from synchronizing the wallet
#[derive(Debug, Clone, Serialize)]
pub struct SyncWalletResponse {}

#[derive(Debug, Clone, Serialize)]
pub enum ReceivePaymentMethod {
    SparkAddress,
    BitcoinAddress,
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub enum SendPaymentMethod {
    BitcoinAddress {
        address: BitcoinAddress,
    },
    Bolt11Invoice {
        detailed_invoice: DetailedBolt11Invoice,
    }, // should be replaced with the parsed invoice
    SparkAddress {
        address: String,
    },
}

pub struct PrepareReceivePaymentRequest {
    pub payment_method: ReceivePaymentMethod,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrepareReceivePaymentResponse {
    pub payment_method: ReceivePaymentMethod,
    pub fee_sats: u64,
}

pub struct ReceivePaymentRequest {
    pub prepare_response: PrepareReceivePaymentResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReceivePaymentResponse {
    pub payment_request: String,
}

pub struct PrepareSendPaymentRequest {
    pub payment_request: String,
    pub amount_sats: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    pub amount_sats: u64,
    pub fee_sats: u64,
}

pub struct SendPaymentRequest {
    pub prepare_response: PrepareSendPaymentResponse,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendPaymentResponse {
    pub payment: Payment,
}

/// Request to list payments with pagination
#[derive(Debug, Clone)]
pub struct ListPaymentsRequest {
    /// Number of records to skip
    pub offset: Option<u32>,
    /// Maximum number of records to return
    pub limit: Option<u32>,
}

/// Response from listing payments
#[derive(Debug, Clone, Serialize)]
pub struct ListPaymentsResponse {
    /// The list of payments
    pub payments: Vec<Payment>,
}

pub struct GetPaymentRequest {
    pub payment_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetPaymentResponse {
    pub payment: Payment,
}

pub trait Logger: Send + Sync {
    fn log(&self, l: LogEntry);
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub line: String,
    pub level: String,
}
