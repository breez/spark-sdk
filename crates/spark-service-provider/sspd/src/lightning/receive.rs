use std::sync::Arc;
use std::time::{Duration, SystemTime};

use std::str::FromStr;

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use lightning::routing::gossip::RoutingFees;
use lightning::routing::router::{DEFAULT_MAX_TOTAL_CLTV_EXPIRY_DELTA, RouteHint, RouteHintHop};
use lightning_invoice::{Bolt11Invoice, PrivateRoute, RawTaggedField, TaggedField};
use spark::events::{SparkEvent, subscribe_server_events};
use spark::operator::OperatorPool;
use spark::operator::rpc::spark::PreimageRequestRole;
use spark::services::{
    HtlcService, LeafKeyTweak, Preimage, PreimageRequestStatus, PreimageRequestWithTransfer,
    QueryHtlcFilter, Transfer, TransferId, TransferStatus, TransferType,
};
use spark::signer::LeafSigningKey;
use spark::signer::Signer;
use spark::tree::{
    LeavesReservation, ReservationPurpose, ReserveResult, TargetAmounts, TreeNode, TreeNodeId,
    TreeNodeStatus, TreeService, TreeStore,
};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::handover::{HandoverOutcome, HandoverReservation, is_refusal, observe_handover};
use crate::leaves::{LeafSigningKeys, release_reserved_leaves};
use crate::swap::select::decompose_into_powers_of_two;
use crate::wakeup::Wakeup;

use super::node::{
    HeldPayment, InvoiceDescription, LightningNode, LightningNodeError, PaymentState,
};
use super::repository::{HoldInvoiceStatus, LightningReceiveRecord, LightningStore};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const RECEIVE_BACKUP_INTERVAL: Duration = Duration::from_secs(60);

const OPERATOR_EVENT_RECONNECT_DELAY: Duration = Duration::from_secs(5);

const OPERATOR_EVENT_BUFFER: usize = 100;

/// A node checks an invoice's expiry against the times of the blocks it has seen,
/// which trail the clock.
const INVOICE_EXPIRY_GRACE: chrono::Duration = chrono::Duration::hours(3);

const EXPIRED_RECEIVES_PER_PASS: i64 = 100;

/// Half the ten minute block target, so the margin keeps its confidence while
/// blocks come up to twice as fast as the target.
const ASSUMED_BLOCK_INTERVAL: Duration = Duration::from_secs(5 * 60);

const SETTLE_HEADROOM: Duration = Duration::from_secs(10 * 60);

const MARGIN_CONFIDENCE: f64 = 0.99;

/// ldk's `HTLC_FAIL_BACK_BUFFER`: the blocks it subtracts from a held HTLC's
/// `cltv_expiry` to report its `claim_deadline`.
const LDK_HTLC_FAIL_BACK_BUFFER_BLOCKS: u32 = 39;

/// Blocks a held HTLC's claim deadline must be ahead of the tip to wait out `claim_window`
/// and then settle within [`SETTLE_HEADROOM`]. Each adds the [`MARGIN_CONFIDENCE`] quantile
/// of its Poisson block count, at one block per [`ASSUMED_BLOCK_INTERVAL`].
pub fn min_blocks_until_deadline(claim_window: Duration) -> u32 {
    let blocks_over = |window: Duration| {
        let mu = window.as_secs_f64() / ASSUMED_BLOCK_INTERVAL.as_secs_f64();
        poisson_quantile(mu, MARGIN_CONFIDENCE)
    };
    blocks_over(claim_window).saturating_add(blocks_over(SETTLE_HEADROOM))
}

/// The `min_final_cltv_expiry_delta` hold invoices demand, in blocks.
pub fn hold_invoice_cltv_delta(leaf_transfer_expiry: Duration) -> Result<u16, String> {
    if leaf_transfer_expiry.is_zero() {
        return Err("the receive leaf transfer expiry must not be zero".to_string());
    }
    let blocks = min_blocks_until_deadline(leaf_transfer_expiry)
        .saturating_add(LDK_HTLC_FAIL_BACK_BUFFER_BLOCKS);
    if blocks > DEFAULT_MAX_TOTAL_CLTV_EXPIRY_DELTA {
        return Err(format!(
            "a receive leaf transfer expiry of {}s needs invoices demanding {blocks} blocks of \
             CLTV, more than the {DEFAULT_MAX_TOTAL_CLTV_EXPIRY_DELTA} an LDK payer's route \
             carries by default",
            leaf_transfer_expiry.as_secs()
        ));
    }
    u16::try_from(blocks).map_err(|e| e.to_string())
}

fn poisson_quantile(mu: f64, p: f64) -> u32 {
    if mu <= 0.0 {
        return 0;
    }
    let mut term = (-mu).exp();
    let mut cdf = term;
    let mut k: u32 = 0;
    while cdf < p && k < 10_000 {
        k = k.saturating_add(1);
        term *= mu / f64::from(k);
        cdf += term;
    }
    k
}

#[derive(Debug, thiserror::Error)]
pub enum LightningReceiveError {
    #[error("lightning node error: {0}")]
    Node(#[from] LightningNodeError),
    #[error("storage error: {0}")]
    Store(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("payment hash already in use by a different receive request")]
    PaymentHashConflict,
}

pub struct LightningReceiveService {
    node: Arc<dyn LightningNode>,
    store: Arc<dyn LightningStore>,
    invoice_signing_key: Option<SecretKey>,
    cltv_delta: u16,
}

impl LightningReceiveService {
    pub fn new(
        node: Arc<dyn LightningNode>,
        store: Arc<dyn LightningStore>,
        invoice_signing_key: Option<SecretKey>,
        cltv_delta: u16,
    ) -> Self {
        Self {
            node,
            store,
            invoice_signing_key,
            cltv_delta,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn request_lightning_receive(
        &self,
        payment_hash: [u8; 32],
        amount_sats: u64,
        requester: &PublicKey,
        receiver: &PublicKey,
        description: &InvoiceDescription,
        invoice_expiry_secs: u32,
        include_spark_address: bool,
    ) -> Result<LightningReceiveRecord, LightningReceiveError> {
        let payment_hash_hash = sha256::Hash::from_byte_array(payment_hash);

        if let Some(existing) = self
            .store
            .get_receive_by_payment_hash(&payment_hash_hash)
            .await
            .map_err(LightningReceiveError::Store)?
        {
            if existing.requester_identity_public_key == *requester
                && existing.user_identity_public_key == *receiver
                && existing.amount_sats == amount_sats
            {
                return Ok(existing);
            }
            return Err(LightningReceiveError::PaymentHashConflict);
        }

        let now = chrono::Utc::now();
        let expires_at = now
            .checked_add_signed(chrono::Duration::seconds(i64::from(invoice_expiry_secs)))
            .ok_or_else(|| LightningReceiveError::InvalidInput("expiry overflows".to_string()))?;
        if include_spark_address && self.invoice_signing_key.is_none() {
            return Err(LightningReceiveError::InvalidInput(
                "this SSP has no invoice signing key configured, so it cannot advertise a spark \
                 address on an invoice"
                    .to_string(),
            ));
        }

        let amount_msat = if amount_sats == 0 {
            None
        } else {
            Some(amount_sats.checked_mul(1000).ok_or_else(|| {
                LightningReceiveError::InvalidInput("amount overflows msat".to_string())
            })?)
        };
        let encoded_invoice = self
            .node
            .create_hold_invoice(
                &payment_hash,
                amount_msat,
                invoice_expiry_secs,
                description,
                self.cltv_delta,
            )
            .await?;

        // The node's hold invoice API takes no route hints, so the hint is added by
        // signing the invoice again as the node.
        let encoded_invoice = match (&self.invoice_signing_key, include_spark_address) {
            (Some(key), true) => add_spark_address_hint(&encoded_invoice, receiver, key)?,
            _ => encoded_invoice,
        };

        let memo = match description {
            InvoiceDescription::Memo(memo) => Some(memo.clone()),
            InvoiceDescription::Hash(_) => None,
        };
        let record = LightningReceiveRecord {
            id: uuid::Uuid::now_v7().to_string(),
            user_identity_public_key: *receiver,
            requester_identity_public_key: *requester,
            payment_hash: payment_hash_hash,
            amount_sats,
            encoded_invoice,
            transfer_id: None,
            transfer_amount_sats: None,
            reservation: None,
            preimage: None,
            invoice_status: HoldInvoiceStatus::Pending,
            expires_at,
            memo,
            created_at: now,
            updated_at: now,
        };
        self.store
            .insert_receive(&record)
            .await
            .map_err(LightningReceiveError::Store)?;
        Ok(record)
    }
}

/// A route hint hop with this short channel id is not a channel: its `src_node_id`
/// is the receiver's Spark identity key.
const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17_592_187_092_992_000_001;

/// `signing_key` must be the node's key, since an invoice's payee is the key that
/// signs it.
pub fn add_spark_address_hint(
    encoded: &str,
    spark_pubkey: &PublicKey,
    signing_key: &SecretKey,
) -> Result<String, LightningReceiveError> {
    let invoice = Bolt11Invoice::from_str(encoded)
        .map_err(|e| LightningReceiveError::InvalidInput(format!("undecodable invoice: {e}")))?;
    let (mut raw, _, _) = invoice.into_signed_raw().into_parts();
    let hint = RouteHint(vec![RouteHintHop {
        src_node_id: *spark_pubkey,
        short_channel_id: RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID,
        fees: RoutingFees {
            base_msat: 0,
            proportional_millionths: 0,
        },
        cltv_expiry_delta: 0,
        htlc_minimum_msat: None,
        htlc_maximum_msat: None,
    }]);
    let private_route = PrivateRoute::new(hint).map_err(|e| {
        LightningReceiveError::InvalidInput(format!("invalid spark route hint: {e:?}"))
    })?;
    raw.data
        .tagged_fields
        .push(RawTaggedField::KnownSemantics(TaggedField::PrivateRoute(
            private_route,
        )));

    let secp = Secp256k1::signing_only();
    let signed = raw
        .sign(|hash| Ok::<_, ()>(secp.sign_ecdsa_recoverable(hash, signing_key)))
        .map_err(|()| LightningReceiveError::InvalidInput("invoice signing failed".to_string()))?;
    let invoice = Bolt11Invoice::from_signed(signed).map_err(|e| {
        LightningReceiveError::InvalidInput(format!("rebuilt invoice is invalid: {e:?}"))
    })?;
    Ok(invoice.to_string())
}

pub struct ReceiveWorkerDeps {
    pub store: Arc<dyn LightningStore>,
    pub node: Arc<dyn LightningNode>,
    pub operator_pool: Arc<OperatorPool>,
    pub signer: Arc<dyn Signer>,
    pub htlc_service: Arc<HtlcService>,
    pub tree_service: Arc<dyn TreeService>,
    pub tree_store: Arc<dyn TreeStore>,
    pub key_resolver: Arc<dyn LeafSigningKeys>,
    pub network: spark::Network,
    pub wakeup: Wakeup,
    pub leaf_transfer_expiry: Duration,
    pub largest_denomination: u64,
}

pub async fn run_receive_loop(deps: ReceiveWorkerDeps, token: CancellationToken) {
    info!("Starting lightning receive loop");
    loop {
        tokio::select! {
            () = token.cancelled() => {
                info!("Lightning receive loop cancelled");
                return;
            }
            () = deps.wakeup.waited() => {}
            () = tokio::time::sleep(RECEIVE_BACKUP_INTERVAL) => {}
        }
        if let Err(e) = process_pending_receives(&deps).await {
            error!("Lightning receive check failed: {e}");
        }
    }
}

/// Wakes the receive worker on the coordinator's events for preimage swaps the SSP
/// sent, which announce a handover applied or returned, and on every reconnect,
/// since events sent while disconnected are gone.
pub async fn wake_on_handover_events(
    identity_public_key: PublicKey,
    operator_pool: Arc<OperatorPool>,
    wakeup: Wakeup,
    token: CancellationToken,
) {
    let (publisher, mut events) = broadcast::channel(OPERATOR_EVENT_BUFFER);
    let (_subscribed, mut unsubscribe) = watch::channel(());
    let subscription = subscribe_server_events(
        identity_public_key,
        operator_pool,
        &publisher,
        OPERATOR_EVENT_RECONNECT_DELAY,
        &mut unsubscribe,
    );
    let wake = async {
        loop {
            match events.recv().await {
                Ok(SparkEvent::SenderTransfer(transfer))
                    if transfer.transfer_type == TransferType::PreimageSwap =>
                {
                    wakeup.wake();
                }
                Ok(SparkEvent::Connected) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    wakeup.wake();
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    };
    tokio::select! {
        () = token.cancelled() => {}
        () = subscription => {}
        () = wake => {}
    }
}

pub async fn process_pending_receives(deps: &ReceiveWorkerDeps) -> Result<(), BoxError> {
    let paid: Vec<sha256::Hash> = deps
        .node
        .paid_hold_invoices()
        .into_iter()
        .map(sha256::Hash::from_byte_array)
        .collect();
    let expired_before = chrono::Utc::now()
        .checked_sub_signed(INVOICE_EXPIRY_GRACE)
        .ok_or("the clock is out of range")?;
    let mut records = deps.store.unhanded_receives(&paid).await?;
    records.extend(deps.store.handing_over_receives().await?);
    records.extend(
        deps.store
            .unhanded_receives_expired_before(expired_before, EXPIRED_RECEIVES_PER_PASS)
            .await?,
    );
    let mut seen = std::collections::HashSet::new();
    for record in records.into_iter().filter(|r| seen.insert(r.id.clone())) {
        if let Err(e) = process_receive(deps, &record).await {
            error!(receive_id = %record.id, "failed to advance lightning receive: {e}");
        }
    }
    Ok(())
}

async fn process_receive(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
) -> Result<(), BoxError> {
    match (&record.transfer_id, &record.reservation) {
        (None, _) => give_leaves(deps, record).await,
        (Some(transfer_id), Some(reservation)) => {
            settle_handover(deps, record, transfer_id, reservation).await
        }
        (Some(transfer_id), None) => advance_after_give(deps, record, transfer_id).await,
    }
}

enum Incoming {
    Unpaid,
    Held(HeldPayment),
    /// The node fails a payment back once its claim deadline is reached, and still
    /// reports it as pending after that.
    FailedBack,
}

async fn incoming(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
) -> Result<Incoming, BoxError> {
    let Some(held) = deps
        .node
        .incoming_payment(&record.payment_hash.to_byte_array())
        .await?
        .filter(|payment| payment.state == PaymentState::Pending)
        .and_then(|payment| payment.held)
    else {
        return Ok(Incoming::Unpaid);
    };
    let tip = deps.node.current_block_height().await?;
    if tip < held.claim_deadline {
        Ok(Incoming::Held(held))
    } else {
        Ok(Incoming::FailedBack)
    }
}

async fn held_payment(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
) -> Result<Option<HeldPayment>, BoxError> {
    match incoming(deps, record).await? {
        Incoming::Held(held) => Ok(Some(held)),
        Incoming::Unpaid | Incoming::FailedBack => Ok(None),
    }
}

/// The handover is recorded before it is made, so a pass after a crash settles it
/// by its transfer id.
async fn give_leaves(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
) -> Result<(), BoxError> {
    let held = match incoming(deps, record).await? {
        Incoming::Held(held) => held,
        Incoming::FailedBack => {
            info!(receive_id = %record.id, "lightning receive: the node failed the payment back at its claim deadline");
            return cancel_receive(deps, record).await;
        }
        Incoming::Unpaid if expired(record) => return cancel_receive(deps, record).await,
        Incoming::Unpaid => return Ok(()),
    };

    let amount_sats = if record.amount_sats == 0 {
        held.amount_msat / 1000
    } else {
        record.amount_sats
    };
    if !payment_backs_handover(deps, record, amount_sats, held).await? {
        return cancel_receive(deps, record).await;
    }

    let transfer_id = TransferId::generate();
    let reservation = reserve_leaves(deps, amount_sats).await?;
    if let Err(e) = deps
        .store
        .set_receive_handover(
            &record.id,
            &transfer_id,
            amount_sats,
            &HandoverReservation::from(&reservation),
        )
        .await
    {
        // The write may have committed without its answer arriving; a recorded
        // handover keeps its reservation.
        match deps.store.get_receive(&record.id).await {
            Ok(Some(stored)) if stored.transfer_id.is_none() => {
                if let Err(cancel_err) = deps
                    .tree_store
                    .cancel_reservation(&reservation.id, &reservation.leaves)
                    .await
                {
                    error!(reservation_id = %reservation.id, "Failed to cancel reservation: {cancel_err:?}");
                }
            }
            Ok(_) => {}
            Err(read) => {
                error!(reservation_id = %reservation.id, "could not tell whether a handover was recorded, so its leaves stay reserved: {read}");
            }
        }
        return Err(e.into());
    }
    hand_over(deps, record, &transfer_id, amount_sats, &reservation).await
}

fn expired(record: &LightningReceiveRecord) -> bool {
    record
        .expires_at
        .checked_add_signed(INVOICE_EXPIRY_GRACE)
        .is_some_and(|payable_until| payable_until < chrono::Utc::now())
}

async fn payment_backs_handover(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    amount_sats: u64,
    held: HeldPayment,
) -> Result<bool, BoxError> {
    let paid_sats = held.amount_msat / 1000;
    if paid_sats == 0 || paid_sats < amount_sats {
        info!(receive_id = %record.id, paid_sats, "lightning receive: held payment does not cover the receive; refunding");
        return Ok(false);
    }
    let required = min_blocks_until_deadline(deps.leaf_transfer_expiry);
    let tip = deps.node.current_block_height().await?;
    if held.claim_deadline.saturating_sub(tip) < required {
        warn!(
            receive_id = %record.id,
            deadline = held.claim_deadline,
            tip,
            required,
            "lightning receive: incoming HTLC claim deadline is within the safety margin; refunding"
        );
        return Ok(false);
    }
    Ok(true)
}

async fn settle_handover(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    transfer_id: &TransferId,
    reservation: &HandoverReservation,
) -> Result<(), BoxError> {
    let sender = spark::signer::derive_identity_public_key(deps.signer.as_ref()).await?;
    let outcome = observe_handover(&deps.operator_pool, &sender, deps.network, transfer_id).await?;
    match outcome {
        HandoverOutcome::Committed => {
            deps.tree_store
                .finalize_reservation(&reservation.id, None)
                .await?;
            deps.store.clear_receive_reservation(&record.id).await?;
            info!(receive_id = %record.id, "lightning receive: the handover went through");
        }
        HandoverOutcome::RolledBack { settled: true } => {
            release_handover(deps, record, reservation).await?;
            info!(receive_id = %record.id, "lightning receive: the operators rolled the handover back; its leaves are back in the pool");
            if held_payment(deps, record).await?.is_none() {
                return cancel_receive(deps, record).await;
            }
        }
        HandoverOutcome::Undetermined { held: false } => {
            let amount_sats = record
                .transfer_amount_sats
                .ok_or("a recorded handover has no amount")?;
            match held_payment(deps, record).await? {
                Some(held) if payment_backs_handover(deps, record, amount_sats, held).await? => {
                    let reservation = reservation.leaves(deps.tree_store.as_ref()).await?;
                    return hand_over(deps, record, transfer_id, amount_sats, &reservation).await;
                }
                _ => {
                    release_handover(deps, record, reservation).await?;
                    info!(receive_id = %record.id, "lightning receive: gave up a handover no operator has seen, which the payment no longer backs");
                    return cancel_receive(deps, record).await;
                }
            }
        }
        HandoverOutcome::RolledBack { settled: false }
        | HandoverOutcome::Undetermined { held: true } => {
            warn!(receive_id = %record.id, %transfer_id, "lightning receive: waiting for the operators to settle a handover");
        }
    }
    Ok(())
}

async fn release_handover(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    reservation: &HandoverReservation,
) -> Result<(), BoxError> {
    release_reserved_leaves(
        deps.tree_store.as_ref(),
        &reservation.id,
        &reservation.leaf_ids,
    )
    .await?;
    deps.store.clear_receive_handover(&record.id).await?;
    Ok(())
}

/// Settles once the operators have applied the sender key tweak, which puts the
/// leaves out of the SSP's reach, or once the claim deadline is near: a preimage
/// alone can be learned from a `ProvidePreimage` call the operators rolled back.
async fn advance_after_give(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    transfer_id: &TransferId,
) -> Result<(), BoxError> {
    let request = query_htlc(deps, transfer_id).await?.ok_or_else(|| {
        format!("the operators hold no preimage request for the committed handover {transfer_id}")
    })?;
    let preimage = match (&record.preimage, &request.preimage) {
        (Some(preimage), _) => Some(preimage),
        (None, Some(preimage)) if preimage.compute_hash() == record.payment_hash => {
            deps.store
                .set_receive_preimage(&record.id, preimage)
                .await?;
            info!(receive_id = %record.id, "lightning receive: learned preimage");
            Some(preimage)
        }
        (None, Some(_)) => {
            error!(receive_id = %record.id, "lightning receive: the operators returned a preimage that does not match the payment hash");
            None
        }
        (None, None) => None,
    };

    // The operators' `Returned` status, not the local clock, ends the wait: the
    // leaves are the SSP's again, so the payer is refunded even with the preimage
    // known.
    if request.status == PreimageRequestStatus::Returned {
        reclaim_returned_leaves(deps, record, request.transfer.as_ref()).await?;
        return cancel_receive(deps, record).await;
    }
    let Some(preimage) = preimage else {
        return Ok(());
    };
    let tweaked = request
        .transfer
        .as_ref()
        .is_some_and(|transfer| sender_key_tweaked(transfer.status));
    if tweaked || settle_deadline_reached(deps, record).await? {
        return settle_receive(deps, record, preimage).await;
    }
    info!(receive_id = %record.id, "lightning receive: waiting for the operators to apply or return the handover");
    Ok(())
}

fn sender_key_tweaked(status: TransferStatus) -> bool {
    matches!(
        status,
        TransferStatus::SenderKeyTweaked
            | TransferStatus::ReceiverKeyTweaked
            | TransferStatus::ReceiverKeyTweakLocked
            | TransferStatus::ReceiverKeyTweakApplied
            | TransferStatus::ReceiverRefundSigned
            | TransferStatus::Completed
    )
}

/// A handover's expiry is expected to pass before the held payment's claim deadline comes
/// within [`SETTLE_HEADROOM`], so a handover still neither applied nor returned by
/// then most likely went through on an operator that has not caught up.
async fn settle_deadline_reached(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
) -> Result<bool, BoxError> {
    let Some(held) = held_payment(deps, record).await? else {
        return Ok(false);
    };
    let tip = deps.node.current_block_height().await?;
    Ok(held.claim_deadline.saturating_sub(tip) <= min_blocks_until_deadline(Duration::ZERO))
}

async fn settle_receive(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    preimage: &Preimage,
) -> Result<(), BoxError> {
    let payment_hash = record.payment_hash.to_byte_array();
    let payment = deps
        .node
        .incoming_payment(&payment_hash)
        .await?
        .ok_or("the node has no payment for a receive whose leaves were handed over")?;
    if payment.state == PaymentState::Pending && held_payment(deps, record).await?.is_some() {
        let preimage: [u8; 32] = preimage
            .to_vec()
            .try_into()
            .map_err(|_| "stored preimage is not 32 bytes")?;
        deps.node
            .settle_hold_invoice(&payment_hash, &preimage)
            .await?;
    } else if payment.state == PaymentState::Succeeded {
        deps.store
            .set_receive_invoice_status(&record.id, HoldInvoiceStatus::Settled)
            .await?;
        info!(receive_id = %record.id, "lightning receive: settled held invoice");
    } else {
        deps.store
            .set_receive_invoice_status(&record.id, HoldInvoiceStatus::Failed)
            .await?;
        error!(receive_id = %record.id, "lightning receive: the held payment was failed back after the user was paid");
    }
    Ok(())
}

/// Safe once the request is `Returned`, since the operators unlock the leaves in the
/// transaction that marks it. A returned leaf keeps its signing key: only a transfer
/// whose sender key tweak was never applied is returned.
async fn reclaim_returned_leaves(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    transfer: Option<&Transfer>,
) -> Result<(), BoxError> {
    let node_ids: Vec<TreeNodeId> = transfer
        .map(|t| t.leaves.iter().map(|l| l.leaf.id.clone()).collect())
        .unwrap_or_default();
    if node_ids.is_empty() {
        return Ok(());
    }

    let identity_public_key =
        spark::signer::derive_identity_public_key(deps.signer.as_ref()).await?;
    let reclaimed: Vec<TreeNode> = deps
        .tree_service
        .fetch_nodes(&node_ids, false)
        .await?
        .into_iter()
        .filter(|node| {
            node.status == TreeNodeStatus::Available
                && node.owner_identity_public_key == Some(identity_public_key)
        })
        .collect();

    if reclaimed.len() != node_ids.len() {
        warn!(
            receive_id = %record.id,
            reclaimed = reclaimed.len(),
            fronted = node_ids.len(),
            "lightning receive: returned transfer gave back fewer leaves than were fronted"
        );
    }
    if reclaimed.is_empty() {
        return Ok(());
    }
    deps.tree_store.add_leaves(&reclaimed).await?;
    info!(
        receive_id = %record.id,
        count = reclaimed.len(),
        "lightning receive: took the returned transfer's leaves back into the pool"
    );
    Ok(())
}

async fn hand_over(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    transfer_id: &TransferId,
    amount_sats: u64,
    reservation: &LeavesReservation,
) -> Result<(), BoxError> {
    let leaf_key_tweaks = build_leaf_key_tweaks(deps, reservation).await?;

    // The operators drop this expiry when they hold the preimage shares, so it only
    // returns the leaves of a handover whose preimage has not arrived by then.
    let expiry_time = SystemTime::now()
        .checked_add(deps.leaf_transfer_expiry)
        .ok_or("leaf transfer expiry overflows")?;

    let preimage = match deps
        .htlc_service
        .receive_htlc_with_tweaks(
            leaf_key_tweaks,
            &record.user_identity_public_key,
            &record.payment_hash,
            &record.encoded_invoice,
            amount_sats,
            expiry_time,
            Some(transfer_id.clone()),
        )
        .await
    {
        Ok((_transfer, preimage)) => preimage,
        Err(e) if matches!(&e, spark::services::ServiceError::ServiceConnectionError(rpc) if is_refusal(rpc)) =>
        {
            warn!(receive_id = %record.id, "lightning receive: the operators refused the handover: {e}");
            return refused_handover(deps, record, transfer_id, reservation).await;
        }
        Err(e) => return Err(e.into()),
    };
    deps.tree_store
        .finalize_reservation(&reservation.id, None)
        .await?;
    deps.store.clear_receive_reservation(&record.id).await?;

    match preimage {
        Some(preimage) => {
            deps.store
                .set_receive_preimage(&record.id, &preimage)
                .await?;
            info!(receive_id = %record.id, "lightning receive: swap returned preimage; settling");
            deps.wakeup.wake();
        }
        None => {
            info!(receive_id = %record.id, "lightning receive: leaves fronted without preimage (hodl); awaiting preimage");
        }
    }
    Ok(())
}

/// A refused call can still have left copies on the operators, so the leaves are
/// only released once they show none would hand the leaves over.
async fn refused_handover(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
    transfer_id: &TransferId,
    reservation: &LeavesReservation,
) -> Result<(), BoxError> {
    let sender = spark::signer::derive_identity_public_key(deps.signer.as_ref()).await?;
    match observe_handover(&deps.operator_pool, &sender, deps.network, transfer_id).await? {
        HandoverOutcome::Undetermined { held: false }
        | HandoverOutcome::RolledBack { settled: true } => {
            release_handover(deps, record, &HandoverReservation::from(reservation)).await?;
            cancel_receive(deps, record).await
        }
        _ => Ok(()),
    }
}

async fn cancel_receive(
    deps: &ReceiveWorkerDeps,
    record: &LightningReceiveRecord,
) -> Result<(), BoxError> {
    let payment_hash = record.payment_hash.to_byte_array();
    deps.node.cancel_hold_invoice(&payment_hash).await?;
    deps.store
        .set_receive_invoice_status(&record.id, HoldInvoiceStatus::Cancelled)
        .await?;
    info!(receive_id = %record.id, "lightning receive: cancelled held invoice (payer refunded)");
    Ok(())
}

async fn query_htlc(
    deps: &ReceiveWorkerDeps,
    transfer_id: &TransferId,
) -> Result<Option<PreimageRequestWithTransfer>, BoxError> {
    let identity_public_key =
        spark::signer::derive_identity_public_key(deps.signer.as_ref()).await?;
    let result = deps
        .htlc_service
        .query_htlc(
            QueryHtlcFilter {
                transfer_ids: vec![transfer_id.to_string()],
                payment_hashes: Vec::new(),
                identity_public_key,
                status: None,
                // The SSP is the sender of a receive swap.
                match_role: PreimageRequestRole::Sender,
            },
            None,
        )
        .await?;
    Ok(result.items.into_iter().next())
}

async fn reserve_leaves(
    deps: &ReceiveWorkerDeps,
    amount: u64,
) -> Result<LeavesReservation, BoxError> {
    let denominations = decompose_into_powers_of_two(amount, deps.largest_denomination);
    let target = TargetAmounts::new_exact_denominations(denominations);
    match deps
        .tree_store
        .try_reserve_leaves(Some(&target), true, ReservationPurpose::Payment)
        .await?
    {
        ReserveResult::Success(reservation) => Ok(reservation),
        ReserveResult::InsufficientFunds | ReserveResult::WaitForPending { .. } => {
            Err(format!("insufficient pool leaves for amount {amount}").into())
        }
    }
}

async fn build_leaf_key_tweaks(
    deps: &ReceiveWorkerDeps,
    reservation: &LeavesReservation,
) -> Result<Vec<LeafKeyTweak>, BoxError> {
    let mut leaf_key_tweaks = Vec::with_capacity(reservation.leaves.len());
    for node in &reservation.leaves {
        let signing_leaf_id = deps
            .key_resolver
            .get_signing_leaf_id(&node.id.to_string())
            .await
            .map_err(|e| format!("key resolver error: {e}"))?
            .map(|id| id.parse::<TreeNodeId>())
            .transpose()
            .map_err(|e| format!("invalid leaf id: {e}"))?
            .unwrap_or_else(|| node.id.clone());
        leaf_key_tweaks.push(LeafKeyTweak {
            node: node.clone(),
            signing_key: LeafSigningKey {
                derived_from: signing_leaf_id,
            },
        });
    }
    Ok(leaf_key_tweaks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lightning::node::mock::MockLightningNode;
    use crate::lightning::repository::InMemoryLightningStore;

    fn pubkey(byte: u8) -> PublicKey {
        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).expect("secret"))
    }

    #[test]
    fn poisson_quantile_matches_known_cdfs() {
        assert_eq!(poisson_quantile(0.0, 0.99), 0);
        // Poisson(3): CDF(7) = 0.988 < 0.99 <= CDF(8) = 0.996.
        assert_eq!(poisson_quantile(3.0, 0.99), 8);
        // Poisson(1): CDF(3) = 0.981 < 0.99 <= CDF(4) = 0.996.
        assert_eq!(poisson_quantile(1.0, 0.99), 4);
        assert!(poisson_quantile(6.0, 0.50) <= poisson_quantile(6.0, 0.99));
    }

    #[test]
    fn min_blocks_until_deadline_covers_window_and_headroom() {
        let window = Duration::from_secs(30 * 60);
        let expected_mean = (window.as_secs_f64() + SETTLE_HEADROOM.as_secs_f64())
            / ASSUMED_BLOCK_INTERVAL.as_secs_f64();
        assert!(f64::from(min_blocks_until_deadline(window)) > expected_mean);
        assert!(
            min_blocks_until_deadline(Duration::from_secs(60 * 60))
                > min_blocks_until_deadline(window)
        );
    }

    #[test]
    fn hold_invoices_demand_the_cltv_the_configured_expiry_needs() {
        let delta = |secs| hold_invoice_cltv_delta(Duration::from_secs(secs));
        assert!(delta(0).is_err());
        assert_eq!(
            u32::from(delta(1800).unwrap()),
            min_blocks_until_deadline(Duration::from_secs(1800)) + LDK_HTLC_FAIL_BACK_BUFFER_BLOCKS
        );
        assert!(delta(24 * 60 * 60).unwrap() > delta(1800).unwrap());
        assert!(delta(7 * 24 * 60 * 60).is_err());
    }

    #[tokio::test]
    async fn request_creates_invoice_and_is_idempotent() {
        let node: Arc<dyn LightningNode> = Arc::new(MockLightningNode);
        let store: Arc<dyn LightningStore> = Arc::new(InMemoryLightningStore::default());
        let service = LightningReceiveService::new(
            Arc::clone(&node),
            Arc::clone(&store),
            None,
            hold_invoice_cltv_delta(Duration::from_secs(1800)).unwrap(),
        );

        let payment_hash = [7u8; 32];
        let user = pubkey(3);
        let first = service
            .request_lightning_receive(
                payment_hash,
                2_000,
                &user,
                &user,
                &InvoiceDescription::Memo("test".into()),
                3600,
                false,
            )
            .await
            .expect("request");
        assert!(!first.encoded_invoice.is_empty());
        assert_eq!(first.amount_sats, 2_000);
        assert_eq!(store.pending_receives().await.unwrap().len(), 1);

        let second = service
            .request_lightning_receive(
                payment_hash,
                2_000,
                &user,
                &user,
                &InvoiceDescription::Memo("test".into()),
                3600,
                false,
            )
            .await
            .expect("request");
        assert_eq!(first.id, second.id);
        assert_eq!(store.pending_receives().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn request_rejects_payment_hash_taken_by_another_request() {
        let node: Arc<dyn LightningNode> = Arc::new(MockLightningNode);
        let store: Arc<dyn LightningStore> = Arc::new(InMemoryLightningStore::default());
        let service = LightningReceiveService::new(
            Arc::clone(&node),
            Arc::clone(&store),
            None,
            hold_invoice_cltv_delta(Duration::from_secs(1800)).unwrap(),
        );

        let payment_hash = [8u8; 32];
        let alice = pubkey(1);
        service
            .request_lightning_receive(
                payment_hash,
                2_000,
                &alice,
                &alice,
                &InvoiceDescription::Memo("test".into()),
                3600,
                false,
            )
            .await
            .expect("alice's request");

        let bob = pubkey(2);
        let other_user = service
            .request_lightning_receive(
                payment_hash,
                2_000,
                &bob,
                &bob,
                &InvoiceDescription::Memo("test".into()),
                3600,
                false,
            )
            .await;
        assert!(matches!(
            other_user,
            Err(LightningReceiveError::PaymentHashConflict)
        ));

        let other_amount = service
            .request_lightning_receive(
                payment_hash,
                3_000,
                &alice,
                &alice,
                &InvoiceDescription::Memo("test".into()),
                3600,
                false,
            )
            .await;
        assert!(matches!(
            other_amount,
            Err(LightningReceiveError::PaymentHashConflict)
        ));

        assert_eq!(store.pending_receives().await.unwrap().len(), 1);
    }
}
