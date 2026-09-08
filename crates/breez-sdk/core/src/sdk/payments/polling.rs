use platform_utils::time::Duration;
use tracing::debug;

use crate::{
    PaymentStatus, WaitForPaymentIdentifier,
    error::SdkError,
    models::Payment,
    utils::{
        payments::{
            FallbackLookup, fetch_and_process_payment, insert_payment_with_metadata,
            resolve_bolt11_fallback,
        },
        polling::{PollSchedule, poll_until},
    },
};

use super::super::{BreezSdk, helpers::maybe_get_payment_from_storage};

// Polling cadence for wait_for_incoming_payment.
const WAIT_FOR_INCOMING_PAYMENT_INITIAL_DELAY_MS: u64 = 500;
const WAIT_FOR_INCOMING_PAYMENT_MAX_DELAY_MS: u64 = 2000;

pub(super) async fn wait_for_incoming_payment(
    sdk: &BreezSdk,
    identifier: WaitForPaymentIdentifier,
    completion_timeout_secs: u32,
) -> Result<Payment, SdkError> {
    // Fast path: completed payment already in storage.
    if let Some(payment) = maybe_get_payment_from_storage(sdk.storage.as_ref(), &identifier).await?
        && payment.status == PaymentStatus::Completed
    {
        return Ok(payment);
    }

    let schedule = PollSchedule {
        initial_delay: Duration::from_millis(WAIT_FOR_INCOMING_PAYMENT_INITIAL_DELAY_MS),
        max_delay: Duration::from_millis(WAIT_FOR_INCOMING_PAYMENT_MAX_DELAY_MS),
        timeout: Duration::from_secs(completion_timeout_secs.into()),
    };
    let shutdown = Some(sdk.shutdown_sender.subscribe());

    let payment = match identifier {
        WaitForPaymentIdentifier::PaymentId(pid) => {
            poll_until(schedule, shutdown, || {
                fetch_and_process_payment(&sdk.spark_wallet, sdk.storage.clone(), &pid, false)
            })
            .await?
        }
        WaitForPaymentIdentifier::LightningReceive { ssp_id, invoice } => {
            poll_until(schedule, shutdown, || {
                poll_lightning_receive(sdk, &ssp_id, &invoice)
            })
            .await?
        }
    };
    finalize_payment(sdk, payment.clone()).await;
    Ok(payment)
}

/// Wraps `insert_payment_with_metadata` with the LNURL-receive metadata
/// refresh, so an LNURL-receive payment lands in storage with its sender
/// metadata attached. Returns whether a status event was emitted.
pub(super) async fn finalize_payment(sdk: &BreezSdk, mut payment: Payment) -> bool {
    // Must run first: the metadata below is keyed on the Bolt11 invoice, which a
    // Spark-settled payment only carries once this has recovered it.
    resolve_bolt11_fallback(
        &sdk.spark_wallet,
        &sdk.storage,
        &mut payment,
        FallbackLookup::Remote,
    )
    .await;

    // No-op for non-Lightning-receive payments; for LNURL receives
    // this pulls the LNURL metadata into the payment record.
    sdk.sync_single_lnurl_metadata(&mut payment).await;

    insert_payment_with_metadata(
        sdk.spark_wallet.clone(),
        sdk.storage.clone(),
        sdk.event_emitter.clone(),
        payment,
    )
    .await
}

/// Polls for a Bolt11 receive settling over either rail.
///
/// A payer that takes the invoice's Spark destination settles it with a transfer
/// the SSP never sees, leaving the receive request at `InvoiceCreated` for good,
/// so the SSP poll alone would wait out the whole timeout. Background sync
/// ingests that transfer and records it against the Bolt11, so storage is where
/// the Spark-settled payment shows up.
async fn poll_lightning_receive(
    sdk: &BreezSdk,
    ssp_id: &str,
    invoice: &str,
) -> Result<Option<Payment>, SdkError> {
    if let Some(payment) = sdk
        .storage
        .get_payment_by_invoice(invoice.to_string())
        .await?
        && payment.status == PaymentStatus::Completed
    {
        return Ok(Some(payment));
    }
    poll_then_process_lightning_receive(sdk, ssp_id).await
}

/// Polls an inbound Lightning payment by SSP id. The receive object
/// only carries the transfer id once the SSP has forwarded the payment
/// via Spark. Once present, defers to [`fetch_and_process_payment`] to
/// fetch the Spark transfer, claim it, and produce a terminal Payment.
async fn poll_then_process_lightning_receive(
    sdk: &BreezSdk,
    ssp_id: &str,
) -> Result<Option<Payment>, SdkError> {
    let Some(receive) = sdk
        .spark_wallet
        .fetch_lightning_receive_payment(ssp_id)
        .await?
    else {
        debug!("poll_then_process_lightning_receive({ssp_id}): SSP returned no receive yet");
        return Ok(None);
    };
    debug!(
        "poll_then_process_lightning_receive({ssp_id}): SSP status={:?}, transfer_id={:?}",
        receive.status, receive.transfer_id
    );
    let Some(transfer_id) = receive.transfer_id else {
        return Ok(None);
    };
    // Lightning receives are spark transfers from our perspective.
    fetch_and_process_payment(
        &sdk.spark_wallet,
        sdk.storage.clone(),
        &transfer_id.to_string(),
        false,
    )
    .await
}
