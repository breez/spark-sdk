#![allow(clippy::unused_async, clippy::wildcard_imports)]

use std::str::FromStr;
use std::sync::Arc;

use async_graphql::*;
use chrono::Utc;
use spark::services::TransferId;
use uuid::Uuid;

use super::mutation::currency_amount_sats;
use super::require_auth;
use super::scalars::Long;
use super::types::*;
use crate::coop_exit::{check_leaf_count, coop_exit_fees};
use crate::lightning::repository::LightningStore;
use crate::static_deposit::StaticDepositService;

pub struct QueryRoot;

#[Object(rename_fields = "snake_case", rename_args = "snake_case")]
impl QueryRoot {
    #[graphql(complexity = "super::EXTERNAL_CALL_COMPLEXITY + child_complexity")]
    async fn coop_exit_fee_quote(
        &self,
        ctx: &Context<'_>,
        input: CoopExitFeeQuoteInput,
    ) -> Result<CoopExitFeeQuoteOutput> {
        require_auth(ctx)?;
        check_leaf_count(input.leaf_external_ids.len() as u64)
            .map_err(|e| Error::new(e.to_string()))?;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let fee_rates = ctx.data::<Arc<dyn crate::fees::FeeRateSource>>()?;
        let rate = fee_rates
            .sat_per_kw()
            .await
            .map_err(|e| Error::new(format!("fee rate unavailable: {e}")))?;
        let fees = coop_exit_fees(input.leaf_external_ids.len() as u64, rate);
        let user_fee = currency_amount_sats(fees.user_fee_sats);
        let l1_broadcast_fee = currency_amount_sats(fees.l1_broadcast_fee_sats);
        let total_fee = currency_amount_sats(
            fees.l1_broadcast_fee_sats
                .saturating_add(fees.user_fee_sats),
        );
        let now = Utc::now();
        // Fees do not depend on the exit speed.
        let quote = CoopExitFeeQuote {
            id: ID::from(Uuid::now_v7().to_string()),
            created_at: now,
            updated_at: now,
            network,
            total_amount: total_fee,
            user_fee_fast: user_fee.clone(),
            user_fee_medium: user_fee.clone(),
            user_fee_slow: user_fee,
            l1_broadcast_fee_fast: l1_broadcast_fee.clone(),
            l1_broadcast_fee_medium: l1_broadcast_fee.clone(),
            l1_broadcast_fee_slow: l1_broadcast_fee,
            expires_at: now
                .checked_add_signed(chrono::Duration::hours(1))
                .unwrap_or(now),
        };
        Ok(CoopExitFeeQuoteOutput { quote })
    }

    async fn leaves_swap_fee_estimate(
        &self,
        _ctx: &Context<'_>,
        input: LeavesSwapFeeEstimateInput,
    ) -> Result<LeavesSwapFeeEstimateOutput> {
        let _ = input;
        Err("not implemented".into())
    }

    #[graphql(complexity = "super::EXTERNAL_CALL_COMPLEXITY + child_complexity")]
    async fn lightning_send_fee_estimate(
        &self,
        ctx: &Context<'_>,
        input: LightningSendFeeEstimateInput,
    ) -> Result<LightningSendFeeEstimateOutput> {
        require_auth(ctx)?;
        let service = &super::Lightning::services(ctx)?.send;
        let amount_sats = input.amount_sats.and_then(|a| u64::try_from(a.0).ok());
        let fee = service
            .fee_estimate(&input.encoded_invoice, amount_sats)
            .await
            .map_err(|e| Error::new(format!("fee estimate failed: {e}")))?;
        Ok(LightningSendFeeEstimateOutput {
            fee_estimate: super::mutation::currency_amount_sats(fee),
        })
    }

    #[graphql(complexity = "super::EXTERNAL_CALL_COMPLEXITY + child_complexity")]
    async fn static_deposit_quote(
        &self,
        ctx: &Context<'_>,
        input: StaticDepositQuoteInput,
    ) -> Result<StaticDepositQuoteOutput> {
        require_auth(ctx)?;
        let service = ctx.data::<Arc<StaticDepositService>>()?;
        let txid = bitcoin::Txid::from_str(&input.transaction_id)
            .map_err(|e| Error::new(format!("invalid transaction_id: {e}")))?;
        let vout = u32::try_from(input.output_index)
            .map_err(|_| Error::new("output_index must be non-negative"))?;
        let quote = service
            .quote(&txid, vout, spark::Network::from(input.network))
            .await
            .map_err(|e| Error::new(format!("static deposit quote failed: {e}")))?;
        Ok(StaticDepositQuoteOutput {
            transaction_id: input.transaction_id,
            output_index: input.output_index,
            network: input.network,
            credit_amount_sats: Long(i64::try_from(quote.credit_amount_sats).unwrap_or(i64::MAX)),
            signature: quote.signature,
        })
    }

    async fn user_request(&self, ctx: &Context<'_>, request_id: ID) -> Result<Option<UserRequest>> {
        let caller = require_auth(ctx)?;
        let store = ctx.data::<Arc<dyn LightningStore>>()?;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let id = request_id.to_string();
        if let Some(record) = store
            .get_send(&id)
            .await
            .map_err(Error::new)?
            .filter(|r| r.user_identity_public_key == caller)
        {
            return Ok(Some(UserRequest::LightningSendRequest(
                super::lightning::send_record_to_type(&record, network),
            )));
        }
        if let Some(record) = store
            .get_receive(&id)
            .await
            .map_err(Error::new)?
            .filter(|r| {
                r.requester_identity_public_key == caller || r.user_identity_public_key == caller
            })
        {
            return Ok(Some(UserRequest::LightningReceiveRequest(
                super::lightning::receive_record_to_type(&record, network),
            )));
        }
        let coop_exits = ctx.data::<Arc<crate::coop_exit::CoopExitService>>()?;
        if let Some(record) = coop_exits
            .request_for(&id, &caller)
            .await
            .map_err(|e| Error::new(e.to_string()))?
        {
            return Ok(Some(UserRequest::CoopExitRequest(
                super::coop_exit::coop_exit_record_to_type(&record, network),
            )));
        }
        Ok(None)
    }

    #[graphql(complexity = "super::TRANSFERS_COMPLEXITY + child_complexity")]
    async fn transfers(
        &self,
        ctx: &Context<'_>,
        #[graphql(validator(max_items = 1000))] transfer_spark_ids: Vec<Uuid>,
    ) -> Result<Vec<Transfer>> {
        let caller = require_auth(ctx)?;
        let store = ctx.data::<Arc<dyn LightningStore>>()?;
        let static_deposit = ctx.data::<Arc<StaticDepositService>>()?;
        let coop_exits = ctx.data::<Arc<crate::coop_exit::CoopExitService>>()?;
        let network = *ctx.data::<BitcoinNetwork>()?;

        // The SDK treats a transfer left out of the answer as having no SSP request.
        let mut transfers = Vec::new();
        for spark_id in transfer_spark_ids {
            let Ok(transfer_id) = TransferId::from_str(&spark_id.to_string()) else {
                continue;
            };
            if let Some(record) = store
                .get_send_by_transfer_id(&transfer_id)
                .await
                .map_err(Error::new)?
                .filter(|r| r.user_identity_public_key == caller)
            {
                transfers.push(super::lightning::send_transfer_to_type(&record, network));
                continue;
            }
            if let Some(transfer) = store
                .get_receive_by_transfer_id(&transfer_id)
                .await
                .map_err(Error::new)?
                .filter(|r| r.user_identity_public_key == caller)
                .and_then(|record| super::lightning::receive_transfer_to_type(&record, network))
            {
                transfers.push(transfer);
                continue;
            }
            if let Some(transfer) = static_deposit
                .claim_by_transfer_id(&spark_id.to_string())
                .await
                .map_err(Error::new)?
                .filter(|r| r.user_identity_public_key == caller)
                .and_then(|record| super::static_deposit::claim_transfer_to_type(&record, network))
            {
                transfers.push(transfer);
                continue;
            }
            if let Some(record) = coop_exits
                .request_for_transfer(&transfer_id, &caller)
                .await
                .map_err(|e| Error::new(e.to_string()))?
            {
                transfers.push(super::coop_exit::coop_exit_transfer_to_type(
                    &record, network,
                ));
            }
        }
        Ok(transfers)
    }

    async fn wallet_webhooks(&self, _ctx: &Context<'_>) -> Result<ListSparkWalletWebhooksOutput> {
        Err("not implemented".into())
    }
}
