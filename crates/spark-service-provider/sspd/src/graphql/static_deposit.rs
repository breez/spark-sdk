#![allow(clippy::wildcard_imports)]

use async_graphql::ID;
use uuid::Uuid;

use super::lightning::empty_leaves;
use super::mutation::currency_amount_sats;
use super::types::*;
use crate::static_deposit::repository::StaticDepositClaimRecord;

pub(super) fn claim_transfer_to_type(
    record: &StaticDepositClaimRecord,
    network: BitcoinNetwork,
) -> Option<Transfer> {
    let transfer_id = record.transfer_id.as_ref()?;
    let spark_id = Uuid::parse_str(transfer_id).ok();
    let deposit_amount = record.deposit_amount_sats;
    let (status, request_status) = if record.spend_broadcast_txid.is_some() {
        (
            ClaimStaticDepositStatus::SpendTxBroadcast,
            SparkUserRequestStatus::Succeeded,
        )
    } else {
        (
            ClaimStaticDepositStatus::TransferCompleted,
            SparkUserRequestStatus::InProgress,
        )
    };
    Some(Transfer {
        total_amount: currency_amount_sats(record.credit_amount_sats),
        spark_id,
        leaves: empty_leaves(),
        user_request: Some(Box::new(UserRequest::ClaimStaticDeposit(
            ClaimStaticDepositType {
                id: ID::from(record.id.clone()),
                created_at: record.created_at,
                updated_at: record.updated_at,
                network,
                request_status: Some(request_status),
                deposit_amount: currency_amount_sats(deposit_amount),
                credit_amount: currency_amount_sats(record.credit_amount_sats),
                max_fee: currency_amount_sats(
                    deposit_amount.saturating_sub(record.credit_amount_sats),
                ),
                status,
                transaction_id: record.txid.clone(),
                output_index: i32::try_from(record.vout).unwrap_or(0),
                bitcoin_network: network,
                transfer_spark_id: spark_id,
                static_deposit_address: None,
            },
        ))),
    })
}
