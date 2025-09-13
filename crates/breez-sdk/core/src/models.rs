use breez_sdk_common::{
    input::{self, BitcoinAddressDetails, Bolt11InvoiceDetails},
    lnurl::pay::{LnurlPayRequestDetails, SuccessAction, SuccessActionProcessed},
    network::BitcoinNetwork,
};
use core::fmt;
use serde::{Deserialize, Serialize};
use spark_wallet::{
    CoopExitFeeQuote, CoopExitSpeedFeeQuote, ExitSpeed, LightningSendPayment, LightningSendStatus,
    Network as SparkNetwork, SspUserRequest, TransferDirection, TransferStatus, TransferType,
    WalletTransfer,
};
use std::{fmt::Display, str::FromStr, time::UNIX_EPOCH};

use crate::{SdkError, error::DepositClaimError};

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConnectRequest {
    pub config: Config,
    pub mnemonic: String,
    pub storage_dir: String,
}

/// The type of payment
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
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
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentMethod {
    Lightning,
    Spark,
    Deposit,
    Withdraw,
    Unknown,
}

impl Display for PaymentMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentMethod::Lightning => write!(f, "lightning"),
            PaymentMethod::Spark => write!(f, "spark"),
            PaymentMethod::Deposit => write!(f, "deposit"),
            PaymentMethod::Withdraw => write!(f, "withdraw"),
            PaymentMethod::Unknown => write!(f, "unknown"),
        }
    }
}

impl FromStr for PaymentMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lightning" => Ok(PaymentMethod::Lightning),
            "spark" => Ok(PaymentMethod::Spark),
            "deposit" => Ok(PaymentMethod::Deposit),
            "withdraw" => Ok(PaymentMethod::Withdraw),
            "unknown" => Ok(PaymentMethod::Unknown),
            _ => Err(()),
        }
    }
}

impl From<TransferType> for PaymentMethod {
    fn from(value: TransferType) -> Self {
        match value {
            TransferType::PreimageSwap => PaymentMethod::Lightning,
            TransferType::CooperativeExit => PaymentMethod::Withdraw,
            TransferType::Transfer => PaymentMethod::Spark,
            TransferType::UtxoSwap => PaymentMethod::Deposit,
            _ => PaymentMethod::Unknown,
        }
    }
}
/// Represents a payment (sent or received)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
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
    /// Method of payment. Sometimes the payment details is empty so this field
    /// is used to determine the payment method.
    pub method: PaymentMethod,
    /// Details of the payment
    pub details: Option<PaymentDetails>,
}

// TODO: fix large enum variant lint - may be done by boxing lnurl_pay_info but that requires
//  some changes to the wasm bindgen macro
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentDetails {
    Spark,
    Lightning {
        /// Represents the invoice description
        description: Option<String>,
        /// The preimage of the paid invoice (proof of payment).
        preimage: Option<String>,
        /// Represents the Bolt11/Bolt12 invoice associated with a payment
        /// In the case of a Send payment, this is the invoice paid by the user
        /// In the case of a Receive payment, this is the invoice paid to the user
        invoice: String,

        /// The payment hash of the invoice
        payment_hash: String,

        /// The invoice destination/payee pubkey
        destination_pubkey: String,

        /// Lnurl payment information if this was an lnurl payment.
        lnurl_pay_info: Option<LnurlPayInfo>,
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
                let invoice_details = input::parse_invoice(&request.invoice.encoded_invoice)
                    .ok_or(SdkError::Generic(
                        "Invalid invoice in SspUserRequest::LightningReceiveRequest".to_string(),
                    ))?;
                PaymentDetails::Lightning {
                    description: invoice_details.description,
                    preimage: request.lightning_receive_payment_preimage,
                    invoice: request.invoice.encoded_invoice,
                    payment_hash: request.invoice.payment_hash,
                    destination_pubkey: invoice_details.payee_pubkey,
                    lnurl_pay_info: None,
                }
            }
            SspUserRequest::LightningSendRequest(request) => {
                let invoice_details =
                    input::parse_invoice(&request.encoded_invoice).ok_or(SdkError::Generic(
                        "Invalid invoice in SspUserRequest::LightningSendRequest".to_string(),
                    ))?;
                PaymentDetails::Lightning {
                    description: invoice_details.description,
                    preimage: request.lightning_send_payment_preimage,
                    invoice: request.encoded_invoice,
                    payment_hash: invoice_details.payment_hash,
                    destination_pubkey: invoice_details.payee_pubkey,
                    lnurl_pay_info: None,
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
        let (fees_sat, mut amount_sat): (u64, u64) = match transfer.clone().user_request {
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
                    let fee_sat = r.max_fee.as_sats().unwrap_or(0);
                    (fee_sat, transfer.total_value_sat)
                }
                _ => (0, transfer.total_value_sat),
            },
            None => (0, transfer.total_value_sat),
        };

        let details: Option<PaymentDetails> = if let Some(user_request) = transfer.user_request {
            Some(user_request.try_into()?)
        } else {
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
        };

        Ok(Payment {
            id: transfer.id.to_string(),
            payment_type,
            status,
            amount: amount_sat,
            fees: fees_sat,
            timestamp: match transfer.created_at.map(|t| t.duration_since(UNIX_EPOCH)) {
                Some(Ok(duration)) => duration.as_secs(),
                _ => 0,
            },
            method: transfer.transfer_type.into(),
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

        let invoice_details = input::parse_invoice(&payment.encoded_invoice).ok_or(
            SdkError::Generic("Invalid invoice in LightnintSendPayment".to_string()),
        )?;
        let details = PaymentDetails::Lightning {
            description: invoice_details.description,
            preimage: payment.payment_preimage,
            invoice: payment.encoded_invoice,
            payment_hash: invoice_details.payment_hash,
            destination_pubkey: invoice_details.payee_pubkey,
            lnurl_pay_info: None,
        };

        Ok(Payment {
            id: payment.id,
            payment_type: PaymentType::Send,
            status,
            amount: amount_sat,
            fees: payment.fee_sat,
            timestamp: payment.created_at.cast_unsigned(),
            method: PaymentMethod::Lightning,
            details: Some(details),
        })
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
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

impl From<Network> for BitcoinNetwork {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => BitcoinNetwork::Bitcoin,
            Network::Regtest => BitcoinNetwork::Regtest,
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Config {
    pub api_key: Option<String>,
    pub network: Network,
    pub sync_interval_secs: u32,

    // The maximum fee that can be paid for a static deposit claim
    // If not set then any fee is allowed
    pub max_deposit_claim_fee: Option<Fee>,

    /// The domain used for receiving through lnurl-pay and lightning address.
    pub lnurl_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Fee {
    // Fixed fee amount in sats
    Fixed { amount: u64 },
    // Relative fee rate in satoshis per vbyte
    Rate { sat_per_vbyte: u64 },
}

impl Fee {
    pub fn to_sats(&self, vbytes: u64) -> u64 {
        match self {
            Fee::Fixed { amount } => *amount,
            Fee::Rate { sat_per_vbyte } => sat_per_vbyte.saturating_mul(vbytes),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DepositInfo {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: u64,
    pub refund_tx: Option<String>,
    pub refund_tx_id: Option<String>,
    pub claim_error: Option<DepositClaimError>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClaimDepositRequest {
    pub txid: String,
    pub vout: u32,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub max_fee: Option<Fee>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClaimDepositResponse {
    pub payment: Payment,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RefundDepositRequest {
    pub txid: String,
    pub vout: u32,
    pub destination_address: String,
    pub fee: Fee,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RefundDepositResponse {
    pub tx_id: String,
    pub tx_hex: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListUnclaimedDepositsRequest {}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListUnclaimedDepositsResponse {
    pub deposits: Vec<DepositInfo>,
}

impl std::fmt::Display for Fee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fee::Fixed { amount } => write!(f, "Fixed: {amount}"),
            Fee::Rate { sat_per_vbyte } => write!(f, "Rate: {sat_per_vbyte}"),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Request to get the balance of the wallet
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetInfoRequest {}

/// Response containing the balance of the wallet
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetInfoResponse {
    /// The balance in satoshis
    pub balance_sats: u64,
}

/// Request to sync the wallet with the Spark network
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SyncWalletRequest {}

/// Response from synchronizing the wallet
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SyncWalletResponse {}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ReceivePaymentMethod {
    SparkAddress,
    BitcoinAddress,
    Bolt11Invoice {
        description: String,
        amount_sats: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SendPaymentMethod {
    BitcoinAddress {
        address: BitcoinAddressDetails,
        fee_quote: SendOnchainFeeQuote,
    },
    Bolt11Invoice {
        invoice_details: Bolt11InvoiceDetails,
        spark_transfer_fee_sats: Option<u64>,
        lightning_fee_sats: u64,
    }, // should be replaced with the parsed invoice
    SparkAddress {
        address: String,
        fee_sats: u64,
    },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct SendOnchainFeeQuote {
    pub id: String,
    pub expires_at: u64,
    pub speed_fast: SendOnchainSpeedFeeQuote,
    pub speed_medium: SendOnchainSpeedFeeQuote,
    pub speed_slow: SendOnchainSpeedFeeQuote,
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
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize)]
pub struct SendOnchainSpeedFeeQuote {
    pub user_fee_sat: u64,
    pub l1_broadcast_fee_sat: u64,
}

impl SendOnchainSpeedFeeQuote {
    pub fn total_fee_sat(&self) -> u64 {
        self.user_fee_sat.saturating_add(self.l1_broadcast_fee_sat)
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

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceivePaymentRequest {
    pub payment_method: ReceivePaymentMethod,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ReceivePaymentResponse {
    pub payment_request: String,
    pub fee_sats: u64,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareLnurlPayRequest {
    pub amount_sats: u64,
    pub pay_request: LnurlPayRequestDetails,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub comment: Option<String>,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub validate_success_action_url: Option<bool>,
}

#[derive(Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareLnurlPayResponse {
    pub amount_sats: u64,
    pub comment: Option<String>,
    pub pay_request: LnurlPayRequestDetails,
    pub fee_sats: u64,
    pub invoice_details: Bolt11InvoiceDetails,
    pub success_action: Option<SuccessAction>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayRequest {
    pub prepare_response: PrepareLnurlPayResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayResponse {
    pub payment: Payment,
    pub success_action: Option<SuccessActionProcessed>,
}

/// Represents the payment LNURL info
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlPayInfo {
    pub ln_address: Option<String>,
    pub comment: Option<String>,
    pub domain: Option<String>,
    pub metadata: Option<String>,
    pub processed_success_action: Option<SuccessActionProcessed>,
    pub raw_success_action: Option<SuccessAction>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Serialize)]
pub enum OnchainConfirmationSpeed {
    Fast,
    Medium,
    Slow,
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

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareSendPaymentRequest {
    pub payment_request: String,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub amount_sats: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PrepareSendPaymentResponse {
    pub payment_method: SendPaymentMethod,
    pub amount_sats: u64,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SendPaymentOptions {
    BitcoinAddress {
        confirmation_speed: OnchainConfirmationSpeed,
    },
    Bolt11Invoice {
        use_spark: bool,
    },
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SendPaymentRequest {
    pub prepare_response: PrepareSendPaymentResponse,
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub options: Option<SendPaymentOptions>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SendPaymentResponse {
    pub payment: Payment,
}

/// Request to list payments with pagination
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListPaymentsRequest {
    /// Number of records to skip
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub offset: Option<u32>,
    /// Maximum number of records to return
    #[cfg_attr(feature = "uniffi", uniffi(default=None))]
    pub limit: Option<u32>,
}

/// Response from listing payments
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListPaymentsResponse {
    /// The list of payments
    pub payments: Vec<Payment>,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetPaymentRequest {
    pub payment_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetPaymentResponse {
    pub payment: Payment,
}

#[cfg_attr(feature = "uniffi", uniffi::export(callback_interface))]
pub trait Logger: Send + Sync {
    fn log(&self, l: LogEntry);
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LogEntry {
    pub line: String,
    pub level: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckLightningAddressRequest {
    pub username: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetLightningAddressRequest {
    pub username: String,
    pub description: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetLightningAddressResponse {
    pub lnurl: String,
    pub lightning_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecoverLnurlPayResponse {
    pub lnurl: String,
    pub lightning_address: String,
    pub username: String,
    pub description: String,
}
