use std::time::UNIX_EPOCH;

use breez_sdk_common::input;
use spark_wallet::{
    CoopExitFeeQuote, CoopExitSpeedFeeQuote, ExitSpeed, LightningSendPayment, LightningSendStatus,
    Network as SparkNetwork, SspUserRequest, TokenTransactionStatus, TransferDirection,
    TransferStatus, TransferType, WalletTransfer,
};

use crate::{
    Fee, Network, OnchainConfirmationSpeed, Payment, PaymentDetails, PaymentMethod, PaymentStatus,
    PaymentType, SdkError, SendOnchainFeeQuote, SendOnchainSpeedFeeQuote, TokenBalance,
    TokenMetadata,
};

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

impl PaymentStatus {
    pub(crate) fn from_token_transaction_status(
        status: &TokenTransactionStatus,
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
            balance: value.balance.try_into().unwrap_or_default(), // balance will be changed to u128 or similar
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
            max_supply: value.max_supply.try_into().unwrap_or_default(), // max_supply will be changed to u128 or similar
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
