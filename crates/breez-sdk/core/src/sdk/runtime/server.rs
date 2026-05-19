use std::{str::FromStr, time::Duration};

use spark_wallet::{ListTransfersRequest, TransferId};
use tokio::sync::watch;
use tracing::{error, trace};

use crate::{
    GetInfoRequest, GetInfoResponse,
    error::SdkError,
    models::{Payment, PaymentStatus, WaitForPaymentIdentifier},
    persist::ObjectCacheRepository,
    utils::{
        polling::{PollSchedule, poll_until},
        token::token_transaction_to_payments,
    },
};

use super::{RuntimeEvent, RuntimeProfile};
use crate::sdk::{BreezSdk, SyncType, helpers::maybe_get_payment_from_storage};
use crate::utils::payments::get_payment_and_emit_event;

// Polling cadence for server-mode wait_for_payment.
const POLL_INITIAL_DELAY_MS: u64 = 500;
const POLL_MAX_DELAY_MS: u64 = 2000;

pub(super) struct ServerRuntime;

#[macros::async_trait]
impl RuntimeProfile for ServerRuntime {
    fn starts_background_services(&self) -> bool {
        false
    }

    async fn start_sdk_services(&self, sdk: &BreezSdk, initial_synced_sender: watch::Sender<bool>) {
        if let Err(e) = initial_synced_sender.send(true) {
            error!("Failed to set initial synced signal in server mode: {e:?}");
        }

        sdk.event_emitter
            .add_runtime_event_handler(Box::new(ServerRuntimeEventHandler { sdk: sdk.clone() }))
            .await;
    }

    async fn run_user_sync(
        &self,
        sdk: &BreezSdk,
        sync_type: SyncType,
        force: bool,
    ) -> Result<(), SdkError> {
        sdk.sync_wallet_internal(sync_type, force).await
    }

    async fn get_info(
        &self,
        sdk: &BreezSdk,
        request: GetInfoRequest,
    ) -> Result<GetInfoResponse, SdkError> {
        if request.ensure_synced.unwrap_or_default() {
            return Err(SdkError::InvalidInput(
                "ensure_synced is not supported when background_tasks_enabled is false; call sync_wallet explicitly instead".to_string(),
            ));
        }

        let (balance_sats, token_balances) = tokio::try_join!(
            sdk.spark_wallet.get_balance(),
            sdk.spark_wallet.get_token_balances(),
        )?;

        let token_balances = token_balances
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();

        Ok(GetInfoResponse {
            identity_pubkey: sdk.spark_wallet.get_identity_public_key().to_string(),
            balance_sats,
            token_balances,
        })
    }

    async fn maybe_ensure_spark_private_mode_initialized(
        &self,
        _sdk: &BreezSdk,
    ) -> Result<(), SdkError> {
        Ok(())
    }

    async fn wait_for_payment(
        &self,
        sdk: &BreezSdk,
        identifier: WaitForPaymentIdentifier,
        completion_timeout_secs: u32,
    ) -> Result<Payment, SdkError> {
        // Fast path: completed payment already in storage.
        if let Some(payment) =
            maybe_get_payment_from_storage(sdk.storage.as_ref(), &identifier).await?
            && payment.status == PaymentStatus::Completed
        {
            return Ok(payment);
        }

        let schedule = PollSchedule {
            initial_delay: Duration::from_millis(POLL_INITIAL_DELAY_MS),
            max_delay: Duration::from_millis(POLL_MAX_DELAY_MS),
            timeout: Duration::from_secs(completion_timeout_secs.into()),
        };
        let shutdown = Some(sdk.shutdown_sender.subscribe());

        let payment = match identifier {
            WaitForPaymentIdentifier::PaymentId(pid) => {
                // Mirrors fetch_payment_id_by_identifier (flashnet.rs:341):
                // a TransferId-shaped payment_id is a Spark transfer;
                // otherwise it's `{token_tx_hash}:{vout}`.
                if let Ok(transfer_id) = TransferId::from_str(&pid) {
                    poll_until(schedule, shutdown, || {
                        probe_spark_transfer(sdk, transfer_id.clone())
                    })
                    .await?
                } else if let Some((hash, _vout)) = pid.split_once(':') {
                    let tx_hash = hash.to_string();
                    poll_until(schedule, shutdown, || {
                        probe_token_transaction(sdk, &pid, &tx_hash)
                    })
                    .await?
                } else {
                    return Err(SdkError::Generic(format!(
                        "Unrecognized payment_id format: {pid}"
                    )));
                }
            }
            WaitForPaymentIdentifier::PaymentRequest(invoice) => {
                // Invoice probe runs sync_wallet_internal on each iteration;
                // the sync claims pending transfers and refreshes local
                // stores as a side effect, so no post-poll sync is needed.
                return poll_until(schedule, shutdown, || probe_invoice_via_sync(sdk, &invoice))
                    .await;
            }
        };

        // PaymentId probes only confirm operator-side status; trigger a
        // local sync so claim_pending_transfers runs and the new
        // leaves/outputs land in the tree-store / token-output store
        // before downstream callers (e.g. the send leg of a
        // conversion-and-send) try to use them.
        self.run_user_sync(sdk, SyncType::Wallet, true).await?;
        Ok(payment)
    }
}

async fn probe_spark_transfer(
    sdk: &BreezSdk,
    transfer_id: TransferId,
) -> Result<Option<Payment>, SdkError> {
    let mut resp = sdk
        .spark_wallet
        .list_transfers(ListTransfersRequest {
            transfer_ids: vec![transfer_id],
            paging: None,
        })
        .await?;
    let Some(transfer) = resp.items.pop() else {
        return Ok(None);
    };
    let payment: Payment = transfer.try_into()?;
    if payment.status == PaymentStatus::Pending {
        Ok(None)
    } else {
        Ok(Some(payment))
    }
}

async fn probe_token_transaction(
    sdk: &BreezSdk,
    payment_id: &str,
    tx_hash: &str,
) -> Result<Option<Payment>, SdkError> {
    let token_transactions = sdk
        .spark_wallet
        .get_token_transactions_by_hashes(vec![tx_hash.to_string()])
        .await?;
    let Some(token_transaction) = token_transactions.first() else {
        return Ok(None);
    };
    let object_repository = ObjectCacheRepository::new(sdk.storage.clone());
    // `tx_inputs_are_ours: false` is correct for the only current caller
    // (conversion received-leg, where we are the recipient). A future
    // caller waiting on a token tx where we are the sender would need to
    // pass `true` to get the right PaymentType / amount mapping.
    let payments = token_transaction_to_payments(
        &sdk.spark_wallet,
        &object_repository,
        token_transaction,
        false,
    )
    .await?;
    let Some(payment) = payments.into_iter().find(|p| p.id == payment_id) else {
        return Ok(None);
    };
    if payment.status == PaymentStatus::Pending {
        Ok(None)
    } else {
        Ok(Some(payment))
    }
}

async fn probe_invoice_via_sync(
    sdk: &BreezSdk,
    invoice: &str,
) -> Result<Option<Payment>, SdkError> {
    // `force=true` bypasses the "synced recently" skip in
    // sync_wallet_internal — without it, every probe after the first
    // becomes a no-op inside the cache window and we re-read stale
    // storage. `Wallet | WalletState` is the minimal scope that runs
    // claim_pending_transfers and writes Payment records to storage so
    // get_payment_by_invoice can see the incoming send.
    sdk.sync_wallet_internal(SyncType::Wallet | SyncType::WalletState, true)
        .await?;
    let payment = sdk
        .storage
        .get_payment_by_invoice(invoice.to_string())
        .await?;
    let Some(payment) = payment else {
        return Ok(None);
    };
    if payment.status == PaymentStatus::Completed {
        Ok(Some(payment))
    } else {
        trace!(
            "probe_invoice_via_sync: payment status {} not yet completed",
            payment.status
        );
        Ok(None)
    }
}

struct ServerRuntimeEventHandler {
    sdk: BreezSdk,
}

#[macros::async_trait]
impl crate::events::RuntimeEventHandler for ServerRuntimeEventHandler {
    async fn handle(&self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::DepositClaimed { payment } => {
                get_payment_and_emit_event(&self.sdk.storage, &self.sdk.event_emitter, *payment)
                    .await;
            }
            RuntimeEvent::StableBalanceConversionCompleted => {}
        }
    }
}
