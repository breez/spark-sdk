#![allow(clippy::wildcard_imports)]

use async_graphql::ID;
use bitcoin::hashes::Hash;
use uuid::Uuid;

use super::mutation::currency_amount_sats;
use super::scalars::{Hash32, PublicKey};
use super::types::*;
use crate::lightning::repository::{
    LightningReceiveRecord, LightningSendRecord, ReceiveState, SendPaymentStatus,
};

pub(super) fn empty_leaves() -> SparkTransferToLeavesConnection {
    SparkTransferToLeavesConnection {
        count: 0,
        page_info: PageInfo {
            has_next_page: Some(false),
            has_previous_page: Some(false),
            start_cursor: None,
            end_cursor: None,
        },
        entities: Vec::new(),
    }
}

fn transfer_ref(spark_id: Option<&str>, amount_sats: u64) -> Transfer {
    Transfer {
        total_amount: currency_amount_sats(amount_sats),
        spark_id: spark_id.and_then(|id| Uuid::parse_str(id).ok()),
        leaves: empty_leaves(),
        user_request: None,
    }
}

pub fn send_transfer_to_type(record: &LightningSendRecord, network: BitcoinNetwork) -> Transfer {
    Transfer {
        total_amount: currency_amount_sats(record.amount_sats),
        spark_id: Uuid::parse_str(&record.user_transfer_id.to_string()).ok(),
        leaves: empty_leaves(),
        user_request: Some(Box::new(UserRequest::LightningSendRequest(
            send_record_to_type(record, network),
        ))),
    }
}

pub fn receive_transfer_to_type(
    record: &LightningReceiveRecord,
    network: BitcoinNetwork,
) -> Option<Transfer> {
    let transfer_id = record.transfer_id.as_ref()?;
    Some(Transfer {
        total_amount: currency_amount_sats(
            record.transfer_amount_sats.unwrap_or(record.amount_sats),
        ),
        spark_id: Uuid::parse_str(&transfer_id.to_string()).ok(),
        leaves: empty_leaves(),
        user_request: Some(Box::new(UserRequest::LightningReceiveRequest(
            receive_record_to_type(record, network),
        ))),
    })
}

fn send_status(
    record: &LightningSendRecord,
) -> (LightningSendRequestStatus, SparkUserRequestStatus) {
    match record.payment_status {
        SendPaymentStatus::Failed => (
            LightningSendRequestStatus::LightningPaymentFailed,
            SparkUserRequestStatus::Failed,
        ),
        SendPaymentStatus::Succeeded if record.leaves_claimed => (
            LightningSendRequestStatus::TransferCompleted,
            SparkUserRequestStatus::Succeeded,
        ),
        SendPaymentStatus::Succeeded => (
            LightningSendRequestStatus::PreimageProvided,
            SparkUserRequestStatus::InProgress,
        ),
        SendPaymentStatus::Pending if record.ln_payment_id.is_some() => (
            LightningSendRequestStatus::LightningPaymentInitiated,
            SparkUserRequestStatus::InProgress,
        ),
        SendPaymentStatus::Pending => (
            LightningSendRequestStatus::Created,
            SparkUserRequestStatus::Created,
        ),
    }
}

fn receive_status(
    record: &LightningReceiveRecord,
) -> (LightningReceiveRequestStatus, SparkUserRequestStatus) {
    match record.state() {
        // A receive is only marked failed after its handover to the user went
        // through.
        ReceiveState::Settled | ReceiveState::Failed => (
            LightningReceiveRequestStatus::TransferCompleted,
            SparkUserRequestStatus::Succeeded,
        ),
        ReceiveState::Cancelled => (
            LightningReceiveRequestStatus::TransferCanceled,
            SparkUserRequestStatus::Canceled,
        ),
        ReceiveState::ReadyToSettle => (
            LightningReceiveRequestStatus::PaymentPreimageRecovered,
            SparkUserRequestStatus::InProgress,
        ),
        ReceiveState::AwaitingClaimOrReturn => (
            LightningReceiveRequestStatus::TransferCreated,
            SparkUserRequestStatus::InProgress,
        ),
        ReceiveState::AwaitingPayment => (
            LightningReceiveRequestStatus::InvoiceCreated,
            SparkUserRequestStatus::Created,
        ),
    }
}

pub fn send_record_to_type(
    record: &LightningSendRecord,
    network: BitcoinNetwork,
) -> LightningSendRequestType {
    let (status, request_status) = send_status(record);
    let amount_sats = record.amount_sats;
    let fee_sats = record.fee_sats;
    let user_transfer_id = record.user_transfer_id.to_string();
    LightningSendRequestType {
        id: ID::from(record.id.clone()),
        created_at: record.created_at,
        updated_at: record.updated_at,
        network,
        request_status: Some(request_status),
        encoded_invoice: record.encoded_invoice.clone(),
        fee: currency_amount_sats(fee_sats),
        idempotency_key: record.idempotency_key.clone().unwrap_or_default(),
        status,
        transfer: Some(transfer_ref(Some(&user_transfer_id), amount_sats)),
        payment_preimage: record.preimage.as_ref().map(|p| hex::encode(p.to_vec())),
    }
}

pub fn receive_record_to_type(
    record: &LightningReceiveRecord,
    network: BitcoinNetwork,
) -> LightningReceiveRequestType {
    let (status, request_status) = receive_status(record);
    let amount_sats = record.amount_sats;
    let expires_at = record.expires_at;
    let invoice = Invoice {
        encoded_invoice: record.encoded_invoice.clone(),
        bitcoin_network: network,
        payment_hash: Hash32(hex::encode(record.payment_hash.to_byte_array())),
        amount: currency_amount_sats(amount_sats),
        created_at: record.created_at,
        expires_at,
        memo: record.memo.clone(),
    };
    LightningReceiveRequestType {
        id: ID::from(record.id.clone()),
        created_at: record.created_at,
        updated_at: record.updated_at,
        network,
        request_status: Some(request_status),
        invoice,
        status,
        transfer: record.transfer_id.as_ref().map(|id| {
            let id = id.to_string();
            transfer_ref(
                Some(&id),
                record.transfer_amount_sats.unwrap_or(amount_sats),
            )
        }),
        payment_preimage: record
            .preimage
            .as_ref()
            .map(|p| Hash32(hex::encode(p.to_vec()))),
        receiver_identity_public_key: Some(PublicKey(hex::encode(
            record.user_identity_public_key.serialize(),
        ))),
    }
}
