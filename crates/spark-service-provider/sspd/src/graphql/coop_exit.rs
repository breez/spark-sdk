#![allow(clippy::wildcard_imports)]

use async_graphql::ID;
use uuid::Uuid;

use super::mutation::currency_amount_sats;
use super::types::*;
use crate::coop_exit::repository::CoopExitRecord;

fn coop_exit_status(
    record: &CoopExitRecord,
) -> (SparkCoopExitRequestStatus, SparkUserRequestStatus) {
    if !record.completed {
        (
            SparkCoopExitRequestStatus::Initiated,
            SparkUserRequestStatus::Created,
        )
    } else if record.broadcast_txid.is_none() {
        (
            SparkCoopExitRequestStatus::CompleteRequestReceived,
            SparkUserRequestStatus::InProgress,
        )
    } else if !record.leaves_claimed {
        (
            SparkCoopExitRequestStatus::TxBroadcasted,
            SparkUserRequestStatus::InProgress,
        )
    } else {
        (
            SparkCoopExitRequestStatus::Succeeded,
            SparkUserRequestStatus::Succeeded,
        )
    }
}

pub fn coop_exit_transfer_to_type(record: &CoopExitRecord, network: BitcoinNetwork) -> Transfer {
    Transfer {
        user_request: Some(Box::new(UserRequest::CoopExitRequest(
            coop_exit_record_to_type(record, network),
        ))),
        ..transfer(record)
    }
}

fn transfer(record: &CoopExitRecord) -> Transfer {
    Transfer {
        total_amount: currency_amount_sats(record.amount_sats),
        spark_id: Uuid::parse_str(&record.user_transfer_id.to_string()).ok(),
        leaves: SparkTransferToLeavesConnection {
            count: 0,
            page_info: PageInfo {
                has_next_page: Some(false),
                has_previous_page: Some(false),
                start_cursor: None,
                end_cursor: None,
            },
            entities: Vec::new(),
        },
        user_request: None,
    }
}

pub fn coop_exit_record_to_type(
    record: &CoopExitRecord,
    network: BitcoinNetwork,
) -> CoopExitRequestType {
    let (status, request_status) = coop_exit_status(record);
    let transfer_spark_id = Uuid::parse_str(&record.user_transfer_id.to_string()).ok();
    CoopExitRequestType {
        id: ID::from(record.id.clone()),
        created_at: record.created_at,
        updated_at: record.updated_at,
        network,
        request_status: Some(request_status),
        fee: currency_amount_sats(record.fee_sats),
        withdrawal_address: Some(record.withdrawal_address.clone()),
        l1_broadcast_fee: currency_amount_sats(0),
        fee_quote: None,
        exit_speed: None,
        status,
        expires_at: record.expires_at(),
        raw_connector_transaction: hex::encode(&record.raw_connector_tx),
        raw_coop_exit_transaction: hex::encode(&record.raw_coop_exit_tx),
        coop_exit_txid: record.coop_exit_txid.clone(),
        transfer_spark_id,
        transfer: Some(transfer(record)),
    }
}
