use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::PublicKey;
use bitcoin::{ScriptBuf, Sequence, Transaction};
use spark::operator::OperatorPool;
use spark::services::{
    HtlcService, Preimage, ServiceError, Transfer, TransferId, TransferService, TransferStatus,
    TransferType,
};
use spark::signer::{Signer, SignerError};
use spark::utils::htlc_transactions::create_htlc_taproot_address;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::handover::is_refusal;
use crate::leaves::{IncomingLeafStore, IncomingTransfer, LeafSigningKeys, claim_into_pool};
use crate::wakeup::Wakeup;

use super::node::{LightningNode, LightningNodeError, LightningPaymentId, PaymentState};
use super::repository::{LightningSendRecord, LightningStore, SendPaymentStatus};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Node events and new sends wake the worker.
const SEND_BACKUP_INTERVAL: Duration = Duration::from_secs(60);

/// The relative timelock, in blocks, of the sender's reclaim path on a lightning
/// HTLC.
pub const EXPECTED_HTLC_TIMELOCK_BLOCKS: u16 = 2160;

/// A payment can stay pending for its route's whole CLTV and still succeed, so
/// fewer blocks leave more room before the user's transfer expires.
const MAX_ROUTE_CLTV_BLOCKS: u32 = 1008;

/// Blocks are assumed to take up to this long on average, so a payment pending for
/// its whole route still resolves before the transfer expires.
const SLOW_BLOCK_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Time kept between the latest a payment can resolve and the user's transfer
/// expiring, for the SSP to reveal the preimage to the operators.
const PREIMAGE_REVEAL_MARGIN: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum SendValidationError {
    #[error("transfer is not a preimage swap")]
    NotPreimageSwap,
    #[error("transfer is {0}, not waiting on the SSP")]
    NotPending(TransferStatus),
    #[error("transfer has no leaves")]
    NoLeaves,
    #[error("refund transaction is missing its output")]
    MalformedRefundTx,
    #[error("leaf HTLC is not locked to the expected lightning HTLC timeout (refusing to pay)")]
    UnexpectedHtlc,
    #[error("failed to reconstruct the expected HTLC output: {0}")]
    Reconstruction(String),
    #[error("transfer expires before a payment could resolve")]
    ExpiresTooSoon,
    #[error("a leaf's refund timelock has to be renewed before it can be sent")]
    RenewalRequired,
}

/// Returns the most total CLTV, in blocks, the route may take when paying.
pub fn verify_send_transfer(
    transfer: &Transfer,
    ssp_identity_pubkey: &PublicKey,
    payment_hash: &sha256::Hash,
    network: spark::Network,
    now: SystemTime,
) -> Result<u32, SendValidationError> {
    if transfer.transfer_type != TransferType::PreimageSwap {
        return Err(SendValidationError::NotPreimageSwap);
    }
    if !matches!(
        transfer.status,
        TransferStatus::SenderKeyTweakPending | TransferStatus::SenderInitiatedCoordinator
    ) {
        return Err(SendValidationError::NotPending(transfer.status));
    }
    if transfer.leaves.is_empty() {
        return Err(SendValidationError::NoLeaves);
    }

    let expected_htlc_output = htlc_script(
        payment_hash,
        ssp_identity_pubkey,
        &transfer.sender_identity_public_key,
        network,
    )?;

    for leaf in &transfer.leaves {
        if !refund_timelock_survives_claim(leaf.leaf.refund_tx.as_ref()) {
            return Err(SendValidationError::RenewalRequired);
        }
        let refund_txs = [
            Some(&leaf.intermediate_refund_tx),
            leaf.intermediate_direct_refund_tx.as_ref(),
            leaf.intermediate_direct_from_cpfp_refund_tx.as_ref(),
        ];
        for tx in refund_txs.into_iter().flatten() {
            check_refund_output(tx, &expected_htlc_output)?;
        }
    }

    fit_to_expiry(MAX_ROUTE_CLTV_BLOCKS, transfer.expiry_time, now)
}

fn fit_to_expiry(
    cap: u32,
    expiry_time: Option<u64>,
    now: SystemTime,
) -> Result<u32, SendValidationError> {
    let Some(expiry) = expiry_time
        .filter(|secs| *secs > 0)
        .and_then(|secs| UNIX_EPOCH.checked_add(Duration::from_secs(secs)))
    else {
        return Ok(cap);
    };
    let blocks = expiry
        .duration_since(now)
        .ok()
        .and_then(|lifetime| lifetime.checked_sub(PREIMAGE_REVEAL_MARGIN))
        .and_then(|time| time.as_secs().checked_div(SLOW_BLOCK_INTERVAL.as_secs()))
        .unwrap_or(0);
    match cap.min(u32::try_from(blocks).unwrap_or(u32::MAX)) {
        0 => Err(SendValidationError::ExpiresTooSoon),
        cap => Ok(cap),
    }
}

fn htlc_script(
    payment_hash: &sha256::Hash,
    ssp_identity_pubkey: &PublicKey,
    sender_identity_pubkey: &PublicKey,
    network: spark::Network,
) -> Result<ScriptBuf, SendValidationError> {
    create_htlc_taproot_address(
        payment_hash,
        ssp_identity_pubkey,
        Sequence::from_height(EXPECTED_HTLC_TIMELOCK_BLOCKS),
        sender_identity_pubkey,
        network,
    )
    .map_err(|e| SendValidationError::Reconstruction(e.to_string()))
}

/// The operators refuse a receiver's claim of a leaf whose refund timelock is under
/// 200 blocks, even one they accepted into the send.
fn refund_timelock_survives_claim(refund_tx: Option<&Transaction>) -> bool {
    const MIN_CLAIMABLE_TIMELOCK: u32 = 200;
    refund_tx
        .and_then(|tx| tx.input.first())
        .is_some_and(|input| input.sequence.to_consensus_u32() & 0xFFFF >= MIN_CLAIMABLE_TIMELOCK)
}

fn check_refund_output(tx: &Transaction, expected: &ScriptBuf) -> Result<(), SendValidationError> {
    let output = tx
        .output
        .first()
        .ok_or(SendValidationError::MalformedRefundTx)?;
    if output.script_pubkey == *expected {
        Ok(())
    } else {
        Err(SendValidationError::UnexpectedHtlc)
    }
}

/// A policy fee, quoted up front because the node offers no route fee estimate.
#[derive(Debug, Clone, Copy)]
pub struct LightningSendFeePolicy {
    pub base_sats: u64,
    pub ppm: u64,
}

impl LightningSendFeePolicy {
    /// The minimum fee the SSP charges to front a send of `amount_sats`.
    pub fn fee_for(&self, amount_sats: u64) -> u64 {
        let proportional = u128::from(amount_sats)
            .saturating_mul(u128::from(self.ppm))
            .checked_div(1_000_000)
            .unwrap_or(0);
        self.base_sats
            .saturating_add(u64::try_from(proportional).unwrap_or(u64::MAX))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LightningSendError {
    #[error(transparent)]
    Validation(#[from] SendValidationError),
    #[error("lightning node error: {0}")]
    Node(#[from] LightningNodeError),
    #[error("spark service error: {0}")]
    Service(#[from] ServiceError),
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("storage error: {0}")]
    Store(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("an amount is required for an amountless invoice")]
    AmountRequired,
    #[error("transfer not found: {0}")]
    TransferNotFound(String),
    #[error("the transfer or idempotency key belongs to a different send request")]
    RequestConflict,
    #[error("the invoice is already being paid")]
    AlreadyPaying,
}

fn idempotent_send_or_conflict(
    existing: LightningSendRecord,
    caller: &PublicKey,
    user_transfer_id: &TransferId,
    encoded_invoice: &str,
) -> Result<LightningSendRecord, LightningSendError> {
    if existing.user_identity_public_key == *caller
        && existing.user_transfer_id == *user_transfer_id
        && existing.encoded_invoice == encoded_invoice
    {
        Ok(existing)
    } else {
        Err(LightningSendError::RequestConflict)
    }
}

/// Rounds a fraction of a sat up, so what the SSP pays stays within what the user
/// commits.
fn send_amount_sats(
    invoice_amount_msat: Option<u64>,
    amount_sats: Option<u64>,
) -> Result<u64, LightningSendError> {
    match (invoice_amount_msat, amount_sats) {
        (Some(msat), _) => Ok(msat.div_ceil(1000)),
        (None, Some(sats)) if sats > 0 => Ok(sats),
        (None, _) => Err(LightningSendError::AmountRequired),
    }
}

pub struct LightningSendService {
    node: Arc<dyn LightningNode>,
    store: Arc<dyn LightningStore>,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    transfer_service: Arc<TransferService>,
    network: spark::Network,
    fee_policy: LightningSendFeePolicy,
    wakeup: Wakeup,
}

impl LightningSendService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node: Arc<dyn LightningNode>,
        store: Arc<dyn LightningStore>,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
        transfer_service: Arc<TransferService>,
        network: spark::Network,
        fee_policy: LightningSendFeePolicy,
        wakeup: Wakeup,
    ) -> Self {
        Self {
            node,
            store,
            operator_pool,
            signer,
            transfer_service,
            network,
            fee_policy,
            wakeup,
        }
    }

    pub async fn fee_estimate(
        &self,
        encoded_invoice: &str,
        amount_sats: Option<u64>,
    ) -> Result<u64, LightningSendError> {
        let decoded = self.node.decode_invoice(encoded_invoice).await?;
        let amount_sats = send_amount_sats(decoded.amount_msat, amount_sats)?;
        Ok(self.fee_policy.fee_for(amount_sats))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn request_lightning_send(
        &self,
        caller: &PublicKey,
        encoded_invoice: &str,
        amount_sats: Option<u64>,
        idempotency_key: Option<&str>,
        user_transfer_id: &str,
    ) -> Result<LightningSendRecord, LightningSendError> {
        let transfer_id = TransferId::from_str(user_transfer_id)
            .map_err(|e| LightningSendError::InvalidInput(format!("invalid transfer id: {e}")))?;
        let existing = match self
            .store
            .get_send_by_transfer_id(&transfer_id)
            .await
            .map_err(LightningSendError::Store)?
        {
            Some(record) => Some(record),
            None => match idempotency_key {
                Some(key) => self
                    .store
                    .get_send_by_idempotency_key(key)
                    .await
                    .map_err(LightningSendError::Store)?,
                None => None,
            },
        };
        if let Some(record) = existing {
            return idempotent_send_or_conflict(record, caller, &transfer_id, encoded_invoice);
        }

        let decoded = self.node.decode_invoice(encoded_invoice).await?;
        let payment_hash = sha256::Hash::from_byte_array(decoded.payment_hash);
        let amount_sats = send_amount_sats(decoded.amount_msat, amount_sats)?;
        // A second send of an invoice would be charged to its user although only
        // one payment goes out.
        if self
            .store
            .get_unfailed_send_by_payment_hash(&payment_hash)
            .await
            .map_err(LightningSendError::Store)?
            .is_some()
        {
            return Err(LightningSendError::AlreadyPaying);
        }

        let incoming = IncomingTransfer::query(
            &self.operator_pool,
            self.signer.as_ref(),
            self.network,
            &transfer_id,
        )
        .await?
        .ok_or_else(|| LightningSendError::TransferNotFound(transfer_id.to_string()))?;
        let transfer = &incoming.transfer;

        let ssp_pubkey = spark::signer::derive_identity_public_key(self.signer.as_ref()).await?;
        if transfer.receiver_identity_public_key != ssp_pubkey {
            return Err(LightningSendError::InvalidInput(
                "transfer is not addressed to the SSP".to_string(),
            ));
        }
        if transfer.sender_identity_public_key != *caller {
            return Err(LightningSendError::InvalidInput(
                "transfer was not sent by the caller".to_string(),
            ));
        }

        let htlc_address = bitcoin::Address::from_script(
            &htlc_script(&payment_hash, &ssp_pubkey, caller, self.network)?,
            bitcoin::Network::from(self.network),
        )
        .map_err(|e| LightningSendError::InvalidInput(format!("invalid HTLC output: {e}")))?
        .to_string();
        // The route cap is derived again when the worker pays, against the
        // transfer as it is then.
        verify_send_transfer(
            transfer,
            &ssp_pubkey,
            &payment_hash,
            self.network,
            SystemTime::now(),
        )?;
        if !incoming
            .is_claimable(&self.transfer_service, self.signer.as_ref())
            .await
        {
            return Err(LightningSendError::InvalidInput(
                "the SSP could not claim the transfer's leaves".to_string(),
            ));
        }

        let min_fee = self.fee_policy.fee_for(amount_sats);
        let required = amount_sats.saturating_add(min_fee);
        let committed: u64 = transfer.leaves.iter().map(|leaf| leaf.leaf.value).sum();
        if committed < required {
            return Err(LightningSendError::InvalidInput(format!(
                "committed leaves {committed} do not cover the send amount {amount_sats} plus fee {min_fee}"
            )));
        }
        let fee_sats = committed.saturating_sub(amount_sats);

        let record = LightningSendRecord {
            id: uuid::Uuid::now_v7().to_string(),
            user_identity_public_key: *caller,
            encoded_invoice: encoded_invoice.to_string(),
            payment_hash,
            amount_sats,
            fee_sats,
            user_transfer_id: transfer_id,
            htlc_address,
            idempotency_key: idempotency_key.map(str::to_string),
            ln_payment_id: None,
            preimage: None,
            payment_status: SendPaymentStatus::Pending,
            leaves_claimed: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        self.store
            .insert_send(&record)
            .await
            .map_err(LightningSendError::Store)?;
        self.wakeup.wake();
        Ok(record)
    }
}

pub struct SendWorkerDeps {
    pub store: Arc<dyn LightningStore>,
    pub node: Arc<dyn LightningNode>,
    pub operator_pool: Arc<OperatorPool>,
    pub signer: Arc<dyn Signer>,
    pub transfer_service: Arc<TransferService>,
    pub htlc_service: Arc<HtlcService>,
    pub incoming: Arc<dyn IncomingLeafStore>,
    pub admission: Wakeup,
    pub leaf_signing_keys: Arc<dyn LeafSigningKeys>,
    pub network: spark::Network,
    pub wakeup: Wakeup,
}

pub async fn run_send_loop(deps: SendWorkerDeps, token: CancellationToken) {
    info!("Starting lightning send loop");
    loop {
        tokio::select! {
            () = token.cancelled() => {
                info!("Lightning send loop cancelled");
                return;
            }
            () = deps.wakeup.waited() => {}
            () = tokio::time::sleep(SEND_BACKUP_INTERVAL) => {}
        }
        if let Err(e) = process_pending_sends(&deps).await {
            error!("Lightning send check failed: {e}");
        }
    }
}

pub async fn process_pending_sends(deps: &SendWorkerDeps) -> Result<(), BoxError> {
    for record in deps.store.pending_sends().await? {
        if let Err(e) = process_send(deps, &record).await {
            error!(send_id = %record.id, "failed to advance lightning send: {e}");
        }
    }
    Ok(())
}

/// Takes the send as far as it can go in one pass.
async fn process_send(deps: &SendWorkerDeps, record: &LightningSendRecord) -> Result<(), BoxError> {
    let mut record = record.clone();
    loop {
        let advanced = match (&record.ln_payment_id, &record.preimage) {
            (None, _) => pay_send(deps, &record).await?,
            (Some(_), None) => poll_send(deps, &record).await?,
            (Some(_), Some(preimage)) => settle_send(deps, preimage, &record).await?,
        };
        if !advanced {
            return Ok(());
        }
        record = deps
            .store
            .get_send(&record.id)
            .await?
            .ok_or("a send disappeared from the store")?;
        if record.is_complete() {
            return Ok(());
        }
    }
}

/// Returns whether the send advanced.
async fn pay_send(deps: &SendWorkerDeps, record: &LightningSendRecord) -> Result<bool, BoxError> {
    // A payment the node has for the invoice, unless it failed without a preimage, is
    // this send's, made by a pass that stopped before recording it: no other send of
    // the invoice is stored while one has such a payment.
    let payment_hash = record.payment_hash.to_byte_array();
    if deps
        .node
        .outgoing_payment(&payment_hash)
        .await?
        .is_some_and(|payment| payment.state != PaymentState::Failed || payment.preimage.is_some())
    {
        deps.store
            .set_send_payment_id(&record.id, &LightningPaymentId(hex::encode(payment_hash)))
            .await?;
        return Ok(true);
    }

    let transfer = deps
        .transfer_service
        .query_transfer(&record.user_transfer_id)
        .await?
        .ok_or_else(|| format!("transfer not found: {}", record.user_transfer_id))?;
    let ssp_pubkey = spark::signer::derive_identity_public_key(deps.signer.as_ref()).await?;
    let cltv_cap = match verify_send_transfer(
        &transfer,
        &ssp_pubkey,
        &record.payment_hash,
        deps.network,
        SystemTime::now(),
    ) {
        Ok(cap) => cap,
        Err(e) => {
            fail_send(deps, record).await?;
            info!(send_id = %record.id, "lightning send: refused to pay ({e}); user keeps the leaves");
            return Ok(true);
        }
    };

    let decoded = deps.node.decode_invoice(&record.encoded_invoice).await?;
    let amount_msat = if decoded.amount_msat.is_some() {
        None
    } else {
        Some(
            record
                .amount_sats
                .checked_mul(1000)
                .ok_or("send amount overflows msat")?,
        )
    };

    // The fee is what the user committed beyond the amount, so a route dearer than
    // it would be paid out of the SSP's own funds.
    let max_routing_fee_msat = record.fee_sats.checked_mul(1000);
    let payment_id = match deps
        .node
        .pay_invoice(
            &record.encoded_invoice,
            amount_msat,
            cltv_cap,
            max_routing_fee_msat,
        )
        .await
    {
        Ok(payment_id) => payment_id,
        Err(LightningNodeError::PaymentSendingFailed(reason)) => {
            fail_send(deps, record).await?;
            info!(send_id = %record.id, "lightning send: payment failed to send ({reason}); user keeps the leaves");
            return Ok(true);
        }
        Err(e) => return Err(e.into()),
    };
    deps.store
        .set_send_payment_id(&record.id, &payment_id)
        .await?;
    info!(send_id = %record.id, "lightning send: payment initiated");
    Ok(true)
}

/// Hands the user's leaves back rather than leaving them locked until the transfer
/// expires. The operators refuse to return a transfer that is no longer pending,
/// whose leaves are then already where they belong.
async fn fail_send(deps: &SendWorkerDeps, record: &LightningSendRecord) -> Result<(), BoxError> {
    match crate::operator_rpc::return_stuck_transfer(
        &deps.operator_pool.get_coordinator().client,
        crate::operator_rpc::ReturnStuckTransferRequest {
            transfer_id: record.user_transfer_id.to_string(),
        },
    )
    .await
    {
        Ok(_) => {}
        Err(e) if is_refusal(&e) => {
            warn!(send_id = %record.id, "lightning send: the operators would not return the user's transfer: {e}");
        }
        Err(e) => return Err(e.into()),
    }
    deps.store.set_send_failed(&record.id).await?;
    Ok(())
}

/// A preimage proves the payment succeeded, whatever state the node reports.
async fn poll_send(deps: &SendWorkerDeps, record: &LightningSendRecord) -> Result<bool, BoxError> {
    let payment = deps
        .node
        .outgoing_payment(&record.payment_hash.to_byte_array())
        .await?
        .ok_or("the node has no payment for a send it initiated")?;
    if let Some(preimage) = payment.preimage {
        let preimage = Preimage::try_from(preimage.to_vec())?;
        deps.store.set_send_succeeded(&record.id, &preimage).await?;
        info!(send_id = %record.id, "lightning send: payment succeeded");
        return Ok(true);
    }
    match payment.state {
        PaymentState::Failed => {
            fail_send(deps, record).await?;
            info!(send_id = %record.id, "lightning send: payment failed; user keeps the leaves");
            Ok(true)
        }
        PaymentState::Succeeded => Err("succeeded lightning payment has no preimage".into()),
        PaymentState::Pending => Ok(false),
    }
}

/// The operators hand over any transfer to the SSP locked to the payment hash, so
/// the preimage is only revealed while this send's own transfer can take it.
async fn settle_send(
    deps: &SendWorkerDeps,
    preimage: &Preimage,
    record: &LightningSendRecord,
) -> Result<bool, BoxError> {
    let transfer = deps
        .transfer_service
        .query_transfer(&record.user_transfer_id)
        .await?
        .ok_or_else(|| format!("transfer not found: {}", record.user_transfer_id))?;
    if matches!(
        transfer.status,
        TransferStatus::Expired | TransferStatus::Returned
    ) {
        error!(send_id = %record.id, status = %transfer.status, "lightning send: the payment succeeded after the user's transfer was returned; the SSP paid it from its own funds");
        return Ok(false);
    }
    let claimed_transfer = deps.htlc_service.provide_preimage(preimage).await?;
    let claimed = claim_into_pool(
        &deps.transfer_service,
        deps.leaf_signing_keys.as_ref(),
        deps.incoming.as_ref(),
        &deps.admission,
        &claimed_transfer,
    )
    .await?;
    deps.store.set_send_leaves_claimed(&record.id).await?;
    info!(send_id = %record.id, count = claimed.len(), "lightning send: claimed user leaves");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use bitcoin::{Amount, OutPoint, TxIn, TxOut, Witness, absolute, transaction};

    fn pubkey(byte: u8) -> PublicKey {
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).expect("secret"))
    }

    fn tx_paying_to(script_pubkey: ScriptBuf) -> Transaction {
        Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                // Irrelevant to the HTLC output match: the timeout lives in the
                // output script.
                sequence: Sequence::from_height(1970),
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey,
            }],
        }
    }

    #[test]
    fn fee_policy_combines_base_and_proportional() {
        let policy = LightningSendFeePolicy {
            base_sats: 5,
            ppm: 2_000,
        };
        assert_eq!(policy.fee_for(1_000_000), 2_005);
        assert_eq!(policy.fee_for(0), 5);
    }

    #[test]
    fn idempotency_returns_same_request_and_rejects_reuse() {
        let transfer_id = TransferId::generate();
        let caller = pubkey(1);
        let record = || LightningSendRecord {
            id: "s1".to_string(),
            user_identity_public_key: pubkey(1),
            encoded_invoice: "inv-A".to_string(),
            payment_hash: sha256::Hash::from_byte_array([9u8; 32]),
            amount_sats: 1_000,
            fee_sats: 10,
            user_transfer_id: transfer_id.clone(),
            htlc_address: "bcrt1phtlc".to_string(),
            idempotency_key: Some("key-1".to_string()),
            ln_payment_id: None,
            preimage: None,
            payment_status: SendPaymentStatus::Pending,
            leaves_claimed: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert!(idempotent_send_or_conflict(record(), &caller, &transfer_id, "inv-A").is_ok());

        assert!(matches!(
            idempotent_send_or_conflict(record(), &caller, &TransferId::generate(), "inv-A"),
            Err(LightningSendError::RequestConflict)
        ));
        assert!(matches!(
            idempotent_send_or_conflict(record(), &caller, &transfer_id, "inv-B"),
            Err(LightningSendError::RequestConflict)
        ));
        assert!(matches!(
            idempotent_send_or_conflict(record(), &pubkey(2), &transfer_id, "inv-A"),
            Err(LightningSendError::RequestConflict)
        ));
    }

    #[test]
    fn a_fraction_of_a_sat_is_rounded_up() {
        assert_eq!(send_amount_sats(Some(1_000_001), Some(5)).unwrap(), 1_001);
        assert_eq!(send_amount_sats(Some(1_000_000), None).unwrap(), 1_000);
        assert_eq!(send_amount_sats(None, Some(5)).unwrap(), 5);
        assert!(matches!(
            send_amount_sats(None, None),
            Err(LightningSendError::AmountRequired)
        ));
        assert!(matches!(
            send_amount_sats(None, Some(0)),
            Err(LightningSendError::AmountRequired)
        ));
    }

    #[test]
    fn the_route_fits_before_the_transfer_expires() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let expiring_in =
            |time: Duration| Some((now + time).duration_since(UNIX_EPOCH).unwrap().as_secs());

        assert_eq!(fit_to_expiry(2_080, None, now).unwrap(), 2_080);
        assert_eq!(fit_to_expiry(2_080, Some(0), now).unwrap(), 2_080);
        let sixty_days = Duration::from_secs(60 * 24 * 60 * 60);
        assert_eq!(
            fit_to_expiry(2_080, expiring_in(sixty_days), now).unwrap(),
            2_080
        );

        let ten_slow_blocks = PREIMAGE_REVEAL_MARGIN + SLOW_BLOCK_INTERVAL * 10;
        assert_eq!(
            fit_to_expiry(2_080, expiring_in(ten_slow_blocks), now).unwrap(),
            10
        );

        assert!(matches!(
            fit_to_expiry(2_080, expiring_in(PREIMAGE_REVEAL_MARGIN), now),
            Err(SendValidationError::ExpiresTooSoon)
        ));
        assert!(matches!(
            fit_to_expiry(2_080, Some(1), now),
            Err(SendValidationError::ExpiresTooSoon)
        ));
    }

    #[test]
    fn a_default_send_transfer_fits_most_routes() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let sixteen_days = (now + Duration::from_secs(16 * 24 * 60 * 60))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(
            fit_to_expiry(MAX_ROUTE_CLTV_BLOCKS, Some(sixteen_days), now).unwrap(),
            720
        );
        assert_eq!(
            fit_to_expiry(MAX_ROUTE_CLTV_BLOCKS, None, now).unwrap(),
            MAX_ROUTE_CLTV_BLOCKS
        );
    }

    #[test]
    fn a_leaf_needing_renewal_is_refused() {
        let refund = |timelock: u16| Transaction {
            input: vec![TxIn {
                sequence: Sequence::from_height(timelock),
                ..TxIn::default()
            }],
            ..tx_paying_to(ScriptBuf::new())
        };
        assert!(!refund_timelock_survives_claim(None));
        assert!(!refund_timelock_survives_claim(Some(&refund(100))));
        assert!(!refund_timelock_survives_claim(Some(&refund(199))));
        assert!(refund_timelock_survives_claim(Some(&refund(200))));
    }

    #[test]
    fn a_different_htlc_timeout_changes_the_output() {
        let ssp = pubkey(1);
        let user = pubkey(2);
        let hash = sha256::Hash::hash(b"payment");
        let net = spark::Network::Regtest;

        let expected = create_htlc_taproot_address(
            &hash,
            &ssp,
            Sequence::from_height(EXPECTED_HTLC_TIMELOCK_BLOCKS),
            &user,
            net,
        )
        .expect("reconstruct");
        let shorter =
            create_htlc_taproot_address(&hash, &ssp, Sequence::from_height(1_000), &user, net)
                .expect("reconstruct");
        assert_ne!(expected, shorter);
    }

    #[test]
    fn matching_output_passes_and_mismatch_fails_closed() {
        let ssp = pubkey(1);
        let user = pubkey(2);
        let hash = sha256::Hash::hash(b"payment");
        let net = spark::Network::Regtest;

        let expected = create_htlc_taproot_address(
            &hash,
            &ssp,
            Sequence::from_height(EXPECTED_HTLC_TIMELOCK_BLOCKS),
            &user,
            net,
        )
        .expect("reconstruct");
        let wrong =
            create_htlc_taproot_address(&hash, &ssp, Sequence::from_height(1_000), &user, net)
                .expect("reconstruct");

        check_refund_output(&tx_paying_to(expected.clone()), &expected).expect("should match");
        assert!(matches!(
            check_refund_output(&tx_paying_to(wrong), &expected),
            Err(SendValidationError::UnexpectedHtlc)
        ));
    }
}
