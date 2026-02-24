use crate::{
    PaymentDetails,
    models::{PaymentStatus, PaymentType},
};
use breez_nostr::{
    error::NostrError,
    model::{PaymentState, Timestamp},
    nips::nip47::model::TransactionType,
};
use nostr::nips::nip47::LookupInvoiceResponse;

impl From<TransactionType> for PaymentType {
    fn from(value: TransactionType) -> PaymentType {
        match value {
            TransactionType::Incoming => PaymentType::Receive,
            TransactionType::Outgoing => PaymentType::Send,
        }
    }
}

impl From<PaymentType> for TransactionType {
    fn from(val: PaymentType) -> Self {
        match val {
            PaymentType::Receive => TransactionType::Incoming,
            PaymentType::Send => TransactionType::Outgoing,
        }
    }
}

impl From<crate::PaymentType> for breez_nostr::model::PaymentType {
    fn from(val: crate::PaymentType) -> Self {
        match val {
            crate::PaymentType::Receive => breez_nostr::model::PaymentType::Incoming,
            crate::PaymentType::Send => breez_nostr::model::PaymentType::Outgoing,
        }
    }
}

impl From<PaymentStatus> for PaymentState {
    fn from(val: PaymentStatus) -> Self {
        match val {
            PaymentStatus::Completed => PaymentState::Complete,
            PaymentStatus::Pending => PaymentState::Pending,
            PaymentStatus::Failed => PaymentState::Failed,
        }
    }
}

impl TryInto<breez_nostr::model::Payment> for crate::Payment {
    type Error = NostrError;

    fn try_into(self) -> Result<breez_nostr::model::Payment, Self::Error> {
        let Some(PaymentDetails::Lightning {
            invoice,
            preimage,
            description,
            payment_hash,
            ..
        }) = self.details
        else {
            return Err(NostrError::generic(
                "Could not convert non-Lightning payment into Nostr payment.",
            ));
        };

        Ok(breez_nostr::model::Payment {
            invoice,
            amount_sat: self
                .amount
                .try_into()
                .ok()
                .and_then(|amount: u64| amount.checked_mul(1_000))
                .ok_or(NostrError::generic("Could not convert payment amount"))?,
            fees_sat: self
                .fees
                .try_into()
                .ok()
                .and_then(|fees: u64| fees.checked_mul(1_000))
                .ok_or(NostrError::generic("Could not convert payment fees"))?,
            timestamp: u32::try_from(self.timestamp).map_err(|err| {
                NostrError::generic(format!("Could not convert payment timestamp: {err}"))
            })?,
            payment_type: self.payment_type.into(),
            payment_state: self.status.into(),
            payment_hash: Some(payment_hash),
            preimage,
            description,
            description_hash: None,
        })
    }
}

impl TryInto<LookupInvoiceResponse> for crate::Payment {
    type Error = NostrError;

    fn try_into(self) -> Result<LookupInvoiceResponse, Self::Error> {
        let Some(PaymentDetails::Lightning {
            invoice,
            preimage,
            description,
            payment_hash,
            ..
        }) = self.details
        else {
            return Err(NostrError::generic(
                "Could not convert non-Lightning payment into Nostr payment.",
            ));
        };

        Ok(LookupInvoiceResponse {
            transaction_type: Some(self.payment_type.into()),
            invoice: Some(invoice),
            description,
            preimage,
            description_hash: None,
            payment_hash,
            amount: self
                .amount
                .try_into()
                .ok()
                .and_then(|amount: u64| amount.checked_mul(1_000))
                .ok_or(NostrError::generic("Could not convert payment amount"))?,
            fees_paid: self
                .fees
                .try_into()
                .ok()
                .and_then(|fees: u64| fees.checked_mul(1_000))
                .ok_or(NostrError::generic("Could not convert payment fees"))?,
            created_at: Timestamp::from_secs(self.timestamp),
            expires_at: None,
            settled_at: None,
            metadata: None,
        })
    }
}
