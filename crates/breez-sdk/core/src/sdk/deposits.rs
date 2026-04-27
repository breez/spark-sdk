use std::{str::FromStr, time::Duration};

use bitcoin::{consensus::serialize, hex::DisplayHex};
use platform_utils::tokio;
use spark_wallet::{ListTransfersRequest, TransferId, WalletTransfer};
use tracing::{error, trace};

use crate::{
    ClaimDepositRequest, ClaimDepositResponse, ListUnclaimedDepositsRequest,
    ListUnclaimedDepositsResponse, RefundDepositRequest, RefundDepositResponse, error::SdkError,
    models::Payment, persist::UpdateDepositPayload, utils::utxo_fetcher::CachedUtxoFetcher,
};

use super::{BreezSdk, SyncType};

// Retry parameters for looking up the transfer created by a static deposit
// claim while it propagates across Spark operators.
const CLAIM_TRANSFER_LOOKUP_MAX_ATTEMPTS: u32 = 3;
const CLAIM_TRANSFER_LOOKUP_BASE_DELAY_MS: u64 = 500;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    pub async fn claim_deposit(
        &self,
        request: ClaimDepositRequest,
    ) -> Result<ClaimDepositResponse, SdkError> {
        self.ensure_spark_private_mode_initialized().await?;
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;

        let max_fee = request
            .max_fee
            .or(self.config.max_deposit_claim_fee.clone());
        match self.claim_utxo(&detailed_utxo, max_fee).await {
            Ok(transfer_id) => {
                let transfer = self.lookup_claim_transfer_with_retry(transfer_id).await?;
                let payment: Payment = transfer.try_into()?;
                // Insert the payment before returning so callers that
                // immediately list payments see the claim.
                self.storage.insert_payment(payment.clone()).await?;
                self.storage
                    .delete_deposit(detailed_utxo.txid.to_string(), detailed_utxo.vout)
                    .await?;
                self.sync_coordinator
                    .trigger_sync_no_wait(SyncType::WalletState, true)
                    .await;
                Ok(ClaimDepositResponse { payment })
            }
            Err(e) => {
                error!("Failed to claim deposit: {e:?}");
                self.storage
                    .update_deposit(
                        detailed_utxo.txid.to_string(),
                        detailed_utxo.vout,
                        UpdateDepositPayload::ClaimError {
                            error: e.clone().into(),
                        },
                    )
                    .await?;
                Err(e)
            }
        }
    }

    pub async fn refund_deposit(
        &self,
        request: RefundDepositRequest,
    ) -> Result<RefundDepositResponse, SdkError> {
        let detailed_utxo =
            CachedUtxoFetcher::new(self.chain_service.clone(), self.storage.clone())
                .fetch_detailed_utxo(&request.txid, request.vout)
                .await?;
        let tx = self
            .spark_wallet
            .refund_static_deposit(
                detailed_utxo.clone().tx,
                Some(detailed_utxo.vout),
                &request.destination_address,
                request.fee.into(),
            )
            .await?;
        let tx_hex = serialize(&tx).as_hex().to_string();
        let tx_id = tx.compute_txid().as_raw_hash().to_string();

        // Store the refund transaction details separately
        self.storage
            .update_deposit(
                detailed_utxo.txid.to_string(),
                detailed_utxo.vout,
                UpdateDepositPayload::Refund {
                    refund_tx: tx_hex.clone(),
                    refund_txid: tx_id.clone(),
                },
            )
            .await?;

        self.chain_service
            .broadcast_transaction(tx_hex.clone())
            .await?;
        Ok(RefundDepositResponse { tx_id, tx_hex })
    }

    #[allow(unused_variables)]
    pub async fn list_unclaimed_deposits(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        let deposits = self.storage.list_deposits().await?;
        Ok(ListUnclaimedDepositsResponse { deposits })
    }
}

impl BreezSdk {
    /// Looks up the transfer produced by a static deposit claim, retrying
    /// while the Spark operators have not yet indexed it. The SSP commits
    /// the claim synchronously, but there is a brief window before the
    /// transfer becomes queryable from the operators; transient query
    /// errors are also retried. Returns the last error if every attempt
    /// fails.
    async fn lookup_claim_transfer_with_retry(
        &self,
        transfer_id: String,
    ) -> Result<WalletTransfer, SdkError> {
        let parsed_id = TransferId::from_str(&transfer_id).map_err(SdkError::Generic)?;
        let mut last_error: Option<SdkError> = None;

        for attempt in 0..CLAIM_TRANSFER_LOOKUP_MAX_ATTEMPTS {
            if attempt > 0 {
                let delay_ms = CLAIM_TRANSFER_LOOKUP_BASE_DELAY_MS
                    .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                trace!(
                    "Retrying claim transfer lookup (attempt {}/{}) for transfer {transfer_id}",
                    attempt.saturating_add(1),
                    CLAIM_TRANSFER_LOOKUP_MAX_ATTEMPTS
                );
            }

            match self
                .spark_wallet
                .list_transfers(ListTransfersRequest {
                    transfer_ids: vec![parsed_id.clone()],
                    paging: None,
                })
                .await
            {
                Ok(mut resp) => {
                    if let Some(transfer) = resp.items.pop() {
                        return Ok(transfer);
                    }
                    last_error = None;
                }
                Err(e) => last_error = Some(e.into()),
            }
        }

        Err(last_error
            .unwrap_or_else(|| SdkError::Generic("transfer not found after claim".to_string())))
    }
}
