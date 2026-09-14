//! Static deposit claims: the SSP transfers pool leaves to a user for their static
//! deposit, then collects the deposit into its own on-chain wallet.

pub mod repository;

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use crate::fees::{self, FeeRateSource};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, ecdsa::Signature};
use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, TapSighash, Transaction, TxOut, Txid, Witness,
};
use spark::bitcoin::sighash_from_tx;
use spark::operator::OperatorPool;
use spark::operator::rpc::spark as pb;
use spark::services::{
    LeafKeyTweak, SigningResult, TransferId, TransferService, UtxoSwapRequestType,
    serialize_instant_static_deposit_claim_payload, serialize_static_deposit_claim_payload,
};
use spark::signer::{
    AggregateFrostRequest, FrostSigningCommitmentsWithNonces, LeafSigningKey, SecretSource,
    SignFrostRequest, Signer,
};
use spark::tree::{
    LeavesReservation, ReservationPurpose, ReserveResult, TargetAmounts, TreeNodeId, TreeStore,
};
use spark::utils::frost::aggregate_frost;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::graphql::types::{ClaimStaticDepositInput, CreateClaimInstantStaticDepositInput};
use crate::handover::{HandoverOutcome, HandoverReservation, is_refusal, observe_handover};
use crate::leaves::{LeafSigningKeys, release_reserved_leaves};
use crate::swap::select::decompose_into_powers_of_two;
use crate::wakeup::Wakeup;

use self::repository::{
    InstantStaticDepositQuoteRecord, InstantStaticDepositQuoteStore, PendingCredit,
    StaticDepositClaimRecord, StaticDepositClaimStore, StaticDepositSpendContext,
    StaticDepositSpendPrep,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Blocks and claims wake the worker.
const STATIC_DEPOSIT_BACKUP_INTERVAL: Duration = Duration::from_secs(60);

const INSTANT_QUOTE_LIFETIME: chrono::Duration = chrono::Duration::hours(1);

/// How long after an INSTANT claim is made its deposit may be missing before the
/// claim is given up.
const DEPOSIT_LOST_AFTER: chrono::Duration = chrono::Duration::weeks(2);

const INSTANT_SECONDARY_CREDIT_AMOUNT_SATS: u64 = 0;

/// The operators refuse to claim an INSTANT swap whose deposit has fewer
/// confirmations.
const INSTANT_CLAIM_MIN_CONFIRMATIONS: u64 = 1;

const SPEND_TX_WEIGHT_WU: u64 = fees::TX_OVERHEAD_WU + fees::P2TR_INPUT_WU + fees::P2TR_OUTPUT_WU;

/// Headroom for the fee rate rising between a quote and its claim, which builds the
/// deposit spend at the rate then.
const QUOTE_FEE_MARGIN_PERCENT: u64 = 20;

#[must_use]
pub fn spend_tx_fee_sats(sat_per_kw: u64) -> u64 {
    fees::fee_sats(sat_per_kw, SPEND_TX_WEIGHT_WU)
}

#[must_use]
pub fn claim_fee_sats(sat_per_kw: u64) -> u64 {
    let spend_cost = spend_tx_fee_sats(sat_per_kw);
    spend_cost.saturating_add(
        spend_cost
            .saturating_mul(QUOTE_FEE_MARGIN_PERCENT)
            .saturating_div(100),
    )
}

/// What an instant claim costs on top of a matured one: the daemon credits the
/// user out of its own funds and holds the deposit's double spend risk until it
/// confirms, which is a risk on the whole amount.
const INSTANT_CLAIM_PREMIUM_BASIS_POINTS: u64 = 10;

#[must_use]
pub fn instant_claim_fee_sats(sat_per_kw: u64, deposit_sats: u64) -> u64 {
    claim_fee_sats(sat_per_kw).saturating_add(
        deposit_sats
            .saturating_mul(INSTANT_CLAIM_PREMIUM_BASIS_POINTS)
            .div_ceil(10_000),
    )
}

fn max_credit_sats(deposit_sats: u64, claim_fee_sats: u64) -> Option<u64> {
    deposit_sats
        .checked_sub(claim_fee_sats)
        .filter(|credit| *credit >= fees::P2TR_DUST_SATS)
}

#[derive(Debug, thiserror::Error)]
pub enum StaticDepositError {
    #[error("signer error: {0}")]
    Signer(#[from] spark::signer::SignerError),
    #[error("tree store error: {0}")]
    TreeStore(#[from] spark::tree::TreeServiceError),
    #[error("spark service error: {0}")]
    Service(#[from] spark::services::ServiceError),
    #[error("operator rpc error: {0}")]
    Rpc(#[from] spark::operator::rpc::OperatorRpcError),
    #[error("storage error: {0}")]
    Store(String),
    #[error("chain error: {0}")]
    Chain(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("insufficient SSP pool leaves for amount {amount}")]
    InsufficientPool { amount: u64 },
    #[error("user signature verification failed")]
    InvalidUserSignature,
    #[error("an earlier claim of this deposit is still settling; retry later")]
    ClaimSettling,
}

pub struct StaticDepositQuote {
    pub credit_amount_sats: u64,
    /// Hex-encoded DER.
    pub signature: String,
}

pub struct DepositOutput {
    pub tx_out: TxOut,
    /// Zero while the deposit is only in the mempool.
    pub confirmations: u64,
}

#[async_trait::async_trait]
pub trait StaticDepositChain: Send + Sync {
    /// The unspent output at `txid:vout`, counting the mempool.
    async fn deposit_output(
        &self,
        txid: &Txid,
        vout: u32,
    ) -> Result<Option<DepositOutput>, BoxError>;
    async fn new_address(&self) -> Result<Address, BoxError>;
    async fn is_confirmed(&self, tx: &Transaction) -> Result<bool, BoxError>;
    /// A transaction the node already has is not an error.
    async fn broadcast(&self, tx: &Transaction) -> Result<(), BoxError>;
}

#[async_trait::async_trait]
pub trait StaticDepositSpendFinalizer: Send + Sync {
    async fn finalize_spend(
        &self,
        record: &StaticDepositClaimRecord,
    ) -> Result<Transaction, BoxError>;
}

#[async_trait::async_trait]
pub trait PendingCreditSettler: Send + Sync {
    async fn settle(
        &self,
        record: &StaticDepositClaimRecord,
        credit: &PendingCredit,
    ) -> Result<(), BoxError>;
}

#[async_trait::async_trait]
impl PendingCreditSettler for StaticDepositService {
    /// Skips a credit whose call is still running in this process: the call may
    /// yet record its answer.
    async fn settle(
        &self,
        record: &StaticDepositClaimRecord,
        credit: &PendingCredit,
    ) -> Result<(), BoxError> {
        let Some(_call) = self.calls.start(&record.id) else {
            return Ok(());
        };
        let sender = spark::signer::derive_identity_public_key(self.signer.as_ref()).await?;
        let outcome = observe_handover(
            &self.operator_pool,
            &sender,
            record.network,
            &credit.transfer_id,
        )
        .await?;
        let reservation = &credit.reservation;
        match outcome {
            HandoverOutcome::Committed if record.is_instant => {
                // The coordinator answers a retry of a reservation it committed with
                // that reservation, matched by its transfer id and signatures, so
                // the retry needs no leaves.
                let transfer = pb::StartTransferRequest {
                    transfer_id: credit.transfer_id.to_string(),
                    owner_identity_public_key: sender.serialize().to_vec(),
                    receiver_identity_public_key: record
                        .user_identity_public_key
                        .serialize()
                        .to_vec(),
                    transfer_package: Some(pb::TransferPackage::default()),
                    ..Default::default()
                };
                let request = reserve_request(record, transfer)?;
                self.reserve(record, credit, request).await?;
            }
            HandoverOutcome::Committed => {
                self.tree_store
                    .finalize_reservation(&reservation.id, None)
                    .await?;
                self.store
                    .set_transfer_id(&record.id, &credit.transfer_id.to_string())
                    .await?;
                info!(claim_id = %record.id, transfer_id = %credit.transfer_id, "static deposit: the user was credited but the claim's answer was lost; the deposit is swept instead");
            }
            HandoverOutcome::RolledBack { settled: true } => {
                self.forget(record, reservation).await?;
                info!(claim_id = %record.id, "static deposit: the operators rolled the credit back; its leaves are back in the pool");
            }
            HandoverOutcome::Undetermined { held: false } => {
                if self.claimed_by_another_swap(record).await? {
                    // Had the swap claiming the deposit been this credit's, the
                    // coordinator would now hold its transfer.
                    let outcome = observe_handover(
                        &self.operator_pool,
                        &sender,
                        record.network,
                        &credit.transfer_id,
                    )
                    .await?;
                    if matches!(outcome, HandoverOutcome::Undetermined { held: false }) {
                        self.forget(record, reservation).await?;
                        info!(claim_id = %record.id, "static deposit: another swap claimed the deposit; the credit's leaves are back in the pool");
                    }
                    return Ok(());
                }
                let leaves = reservation.leaves(self.tree_store.as_ref()).await?;
                if record.is_instant {
                    // The operators credit an INSTANT claim before its deposit
                    // confirms, and the deposit can be replaced meanwhile.
                    if self
                        .chain
                        .deposit_output(&claim_txid(record)?, record.vout)
                        .await?
                        .is_none()
                    {
                        self.forget(record, reservation).await?;
                        info!(claim_id = %record.id, "static deposit: the deposit is gone; the credit's leaves are back in the pool");
                        return Ok(());
                    }
                    let transfer = self
                        .build_credit_transfer(record, &credit.transfer_id, &leaves)
                        .await?;
                    let request = reserve_request(record, transfer)?;
                    self.reserve(record, credit, request).await?;
                } else {
                    let request = self
                        .build_swap_request(record, &credit.transfer_id, &leaves)
                        .await?;
                    self.swap(record, credit, request).await?;
                }
            }
            HandoverOutcome::RolledBack { settled: false }
            | HandoverOutcome::Undetermined { held: true } => {
                warn!(claim_id = %record.id, transfer_id = %credit.transfer_id, "static deposit: waiting for the operators to settle a credit");
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait StaticDepositSpendCosigner: Send + Sync {
    /// Returns whether the claim's spend context was stored.
    async fn cosign_spend(&self, record: &StaticDepositClaimRecord) -> Result<bool, BoxError>;
}

#[derive(Default)]
struct ClaimCalls(std::sync::Mutex<HashSet<String>>);

impl ClaimCalls {
    fn start(&self, id: &str) -> Option<ClaimCall<'_>> {
        let mut running = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        running.insert(id.to_string()).then(|| ClaimCall {
            calls: self,
            id: id.to_string(),
        })
    }
}

struct ClaimCall<'a> {
    calls: &'a ClaimCalls,
    id: String,
}

impl Drop for ClaimCall<'_> {
    fn drop(&mut self) {
        self.calls
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

pub struct StaticDepositService {
    store: Arc<dyn StaticDepositClaimStore>,
    instant_quote_store: Arc<dyn InstantStaticDepositQuoteStore>,
    transfer_service: Arc<TransferService>,
    tree_store: Arc<dyn TreeStore>,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    chain: Arc<dyn StaticDepositChain>,
    fee_rates: Arc<dyn FeeRateSource>,
    key_resolver: Arc<dyn LeafSigningKeys>,
    accept_unconfirmed_deposits: bool,
    largest_denomination: u64,
    wakeup: Wakeup,
    calls: ClaimCalls,
}

impl StaticDepositService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn StaticDepositClaimStore>,
        instant_quote_store: Arc<dyn InstantStaticDepositQuoteStore>,
        transfer_service: Arc<TransferService>,
        tree_store: Arc<dyn TreeStore>,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
        chain: Arc<dyn StaticDepositChain>,
        fee_rates: Arc<dyn FeeRateSource>,
        key_resolver: Arc<dyn LeafSigningKeys>,
        accept_unconfirmed_deposits: bool,
        largest_denomination: u64,
        wakeup: Wakeup,
    ) -> Self {
        Self {
            store,
            instant_quote_store,
            transfer_service,
            tree_store,
            operator_pool,
            signer,
            chain,
            fee_rates,
            key_resolver,
            accept_unconfirmed_deposits,
            largest_denomination,
            wakeup,
            calls: ClaimCalls::default(),
        }
    }

    pub async fn claim_by_transfer_id(
        &self,
        transfer_id: &str,
    ) -> Result<Option<StaticDepositClaimRecord>, String> {
        self.store.get_by_transfer_id(transfer_id).await
    }

    pub async fn quote(
        &self,
        txid: &Txid,
        vout: u32,
        network: spark::Network,
    ) -> Result<StaticDepositQuote, StaticDepositError> {
        let deposit_value = self.deposit_output(txid, vout).await?.tx_out.value.to_sat();
        let fee_sats = self.claim_fee().await?;
        let credit_amount_sats = max_credit_sats(deposit_value, fee_sats).ok_or_else(|| {
            StaticDepositError::InvalidInput(format!(
                "deposit value {deposit_value} does not cover the {fee_sats} sat claim fee"
            ))
        })?;
        let signature = self
            .sign_quote(network, txid, vout, credit_amount_sats)
            .await?;
        Ok(StaticDepositQuote {
            credit_amount_sats,
            signature,
        })
    }

    /// The operators never verify this signature. They allow each one in at most one
    /// uncancelled INSTANT swap, and an INSTANT user statement names no UTXO, so the
    /// signed message names the UTXO.
    async fn sign_quote(
        &self,
        network: spark::Network,
        txid: &Txid,
        vout: u32,
        credit_amount_sats: u64,
    ) -> Result<String, StaticDepositError> {
        let mut message = Vec::new();
        message.extend_from_slice(b"static_deposit_quote");
        message.extend_from_slice(network.to_string().as_bytes());
        message.extend_from_slice(txid.to_string().as_bytes());
        message.extend_from_slice(&vout.to_le_bytes());
        message.extend_from_slice(&credit_amount_sats.to_le_bytes());
        let signature = self
            .signer
            .sign_message_ecdsa(&identity_path(), &message)
            .await?;
        Ok(signature.serialize_der().to_string())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn claim(
        &self,
        input: &ClaimStaticDepositInput,
        user_identity_public_key: PublicKey,
    ) -> Result<String, StaticDepositError> {
        let txid = Txid::from_str(&input.transaction_id).map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid transaction id: {e}"))
        })?;
        let vout = u32::try_from(input.output_index)
            .map_err(|_| StaticDepositError::InvalidInput("invalid output index".to_string()))?;
        let network = spark::Network::from(input.network);
        let credit_amount_sats = input
            .credit_amount_sats
            .map(|a| a.0)
            .and_then(|a| u64::try_from(a).ok())
            .ok_or_else(|| {
                StaticDepositError::InvalidInput(
                    "credit_amount_sats is required and must be non-negative".to_string(),
                )
            })?;

        if let Some(existing) = self
            .store
            .get_by_utxo(&txid.to_string(), vout)
            .await
            .map_err(StaticDepositError::Store)?
        {
            return match existing.transfer_id {
                Some(transfer_id)
                    if !existing.is_instant
                        && existing.user_identity_public_key == user_identity_public_key =>
                {
                    Ok(transfer_id)
                }
                Some(_) => Err(StaticDepositError::InvalidInput(
                    "the deposit was already claimed".to_string(),
                )),
                None => Err(StaticDepositError::ClaimSettling),
            };
        }

        let encrypted_deposit_secret_key = self.resolve_encrypted_deposit_key(input).await?;

        let quote_signature_der = hex::decode(&input.quote_signature).map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid quote_signature hex: {e}"))
        })?;
        let user_signature_der = hex::decode(&input.signature)
            .map_err(|e| StaticDepositError::InvalidInput(format!("invalid signature hex: {e}")))?;

        let statement = serialize_static_deposit_claim_payload(
            network,
            txid,
            vout,
            UtxoSwapRequestType::Fixed,
            credit_amount_sats,
            &quote_signature_der,
        );
        verify_user_ecdsa(&user_identity_public_key, &user_signature_der, &statement)?;

        let deposit = self.deposit_output(&txid, vout).await?;
        let deposit_value = deposit.tx_out.value.to_sat();
        let deposit_address = output_address(&deposit.tx_out, network)?;
        // The client picks the credit, and the operators only cap it at the deposit
        // value.
        let fee_rate = self.fee_rate().await?;
        let fee_sats = spend_tx_fee_sats(fee_rate);
        if max_credit_sats(deposit_value, fee_sats).is_none_or(|max| credit_amount_sats > max) {
            return Err(StaticDepositError::InvalidInput(format!(
                "credit {credit_amount_sats} leaves less than the {fee_sats} sat claim fee \
                 of deposit {deposit_value}; re-quote"
            )));
        }

        // Checked before reserving leaves: the worker may retry a call the operators
        // refuse before taking part in it, holding its leaves meanwhile.
        self.check_deposit_key(
            &encrypted_deposit_secret_key,
            &user_identity_public_key,
            &deposit_address,
            network,
        )
        .await?;
        if !self
            .operators_list(&deposit_address, network, &txid, vout, true)
            .await?
        {
            return Err(StaticDepositError::InvalidInput(
                "the operators do not list the deposit as claimable".to_string(),
            ));
        }
        let spend_prep = self
            .spend_prep(&txid, vout, deposit_value, fee_sats)
            .await?;

        let reservation = self.reserve_leaves(credit_amount_sats).await?;
        let credit = PendingCredit {
            transfer_id: TransferId::generate(),
            reservation: HandoverReservation::from(&reservation),
        };
        let now = chrono::Utc::now();
        let record = StaticDepositClaimRecord {
            id: Uuid::now_v7().to_string(),
            user_identity_public_key,
            txid: txid.to_string(),
            vout,
            network,
            deposit_address,
            credit_amount_sats,
            is_instant: false,
            deposit_amount_sats: deposit_value,
            encrypted_deposit_secret_key,
            quote_signature: input.quote_signature.clone(),
            user_signature: input.signature.clone(),
            transfer_id: None,
            pending_credit: Some(credit.clone()),
            utxo_swap_id: None,
            spend_prep,
            spend_context: None,
            spend_broadcast_txid: None,
            spend_confirmed: false,
            deposit_lost: false,
            created_at: now,
            updated_at: now,
        };
        let call = self.calls.start(&record.id);
        if let Err(e) = self.store.insert(&record).await {
            self.cancel_unless_stored(&record, &reservation).await;
            return Err(StaticDepositError::Store(e));
        }
        let request = match self
            .build_swap_request(&record, &credit.transfer_id, &reservation)
            .await
        {
            Ok(request) => request,
            Err(e) => {
                self.abandon(&record, &reservation).await;
                return Err(e);
            }
        };
        let credited = self.swap(&record, &credit, request).await;
        drop(call);
        self.wakeup.wake();
        credited
    }

    /// An error from the call does not say whether the operators made the transfer,
    /// so the claim keeps its pending credit for the worker to settle.
    async fn swap(
        &self,
        record: &StaticDepositClaimRecord,
        credit: &PendingCredit,
        request: crate::operator_rpc::InitiateStaticDepositUtxoSwapRequest,
    ) -> Result<String, StaticDepositError> {
        let response = match crate::operator_rpc::initiate_static_deposit_utxo_swap(
            &self.operator_pool.get_coordinator().client,
            request,
        )
        .await
        {
            Ok(response) => response,
            Err(e) if is_refusal(&e) => return Err(self.refused(record, credit, e).await),
            Err(e) => return Err(e.into()),
        };
        let reservation_id = &credit.reservation.id;
        if let Err(e) = self
            .tree_store
            .finalize_reservation(reservation_id, None)
            .await
        {
            error!(%reservation_id, "failed to finalize reservation: {e:?}");
        }

        let transfer_id = credit.transfer_id.to_string();
        let spend_context = build_spend_context(
            response
                .deposit_address
                .as_ref()
                .map(|address| address.verifying_public_key.as_slice()),
            response.spend_tx_signing_result,
        );
        match spend_context {
            Ok(context) => {
                self.store
                    .set_spend_context(&record.id, &transfer_id, &context)
                    .await
                    .map_err(StaticDepositError::Store)?;
            }
            Err(e) => {
                if let Err(store_err) = self.store.set_transfer_id(&record.id, &transfer_id).await {
                    error!(claim_id = %record.id, "failed to record transfer id after malformed swap response: {store_err}");
                }
                return Err(e);
            }
        }

        info!(claim_id = %record.id, %transfer_id, "static deposit: credit transfer created");
        Ok(transfer_id)
    }

    /// A refused call can still have left copies on the operators, so the leaves are
    /// only released once they show none would credit the user.
    async fn refused(
        &self,
        record: &StaticDepositClaimRecord,
        credit: &PendingCredit,
        refusal: spark::operator::rpc::OperatorRpcError,
    ) -> StaticDepositError {
        warn!(claim_id = %record.id, "static deposit: the operators refused the credit: {refusal}");
        let outcome = match spark::signer::derive_identity_public_key(self.signer.as_ref()).await {
            Ok(sender) => {
                observe_handover(
                    &self.operator_pool,
                    &sender,
                    record.network,
                    &credit.transfer_id,
                )
                .await
            }
            Err(e) => Err(e.into()),
        };
        match outcome {
            Ok(
                HandoverOutcome::Undetermined { held: false }
                | HandoverOutcome::RolledBack { settled: true },
            ) => {
                if let Err(e) = self.forget(record, &credit.reservation).await {
                    error!(claim_id = %record.id, "failed to give up a refused claim: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => {
                error!(claim_id = %record.id, "could not tell whether a refused credit left copies: {e}");
            }
        }
        StaticDepositError::Rpc(refusal)
    }

    async fn forget(
        &self,
        record: &StaticDepositClaimRecord,
        reservation: &HandoverReservation,
    ) -> Result<(), StaticDepositError> {
        release_reserved_leaves(
            self.tree_store.as_ref(),
            &reservation.id,
            &reservation.leaf_ids,
        )
        .await?;
        self.store
            .delete(&record.id)
            .await
            .map_err(StaticDepositError::Store)
    }

    pub async fn instant_quote(
        &self,
        txid: &Txid,
        vout: u32,
        network: spark::Network,
    ) -> Result<InstantStaticDepositQuoteRecord, StaticDepositError> {
        let output = self.deposit_output(txid, vout).await?;
        self.check_confirmations(&output)?;
        let deposit_amount_sats = output.tx_out.value.to_sat();
        let fee_sats = instant_claim_fee_sats(self.fee_rate().await?, deposit_amount_sats);
        let credit_amount_sats =
            max_credit_sats(deposit_amount_sats, fee_sats).ok_or_else(|| {
                StaticDepositError::InvalidInput(format!(
                    "deposit value {deposit_amount_sats} does not cover the {fee_sats} sat claim fee"
                ))
            })?;
        let destination_address = output_address(&output.tx_out, network)?;
        let quote_signature = self
            .sign_quote(network, txid, vout, credit_amount_sats)
            .await?;
        let now = chrono::Utc::now();
        let record = InstantStaticDepositQuoteRecord {
            id: Uuid::now_v7().to_string(),
            txid: txid.to_string(),
            vout,
            network,
            deposit_amount_sats,
            credit_amount_sats,
            destination_address,
            quote_signature,
            created_at: now,
            updated_at: now,
        };
        self.instant_quote_store
            .insert(&record)
            .await
            .map_err(StaticDepositError::Store)?;
        Ok(record)
    }

    #[allow(clippy::too_many_lines)]
    pub async fn claim_instant(
        &self,
        input: &CreateClaimInstantStaticDepositInput,
        user_identity_public_key: PublicKey,
    ) -> Result<String, StaticDepositError> {
        let quote_id = input.static_deposit_quote_id.to_string();
        let quote = self
            .instant_quote_store
            .get(&quote_id)
            .await
            .map_err(StaticDepositError::Store)?
            .ok_or_else(|| {
                StaticDepositError::InvalidInput(format!("unknown instant quote id {quote_id}"))
            })?;
        if quote
            .created_at
            .checked_add_signed(INSTANT_QUOTE_LIFETIME)
            .is_none_or(|expiry| expiry < chrono::Utc::now())
        {
            return Err(StaticDepositError::InvalidInput(
                "the quote expired; re-quote".to_string(),
            ));
        }
        let txid = Txid::from_str(&quote.txid).map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid quote transaction id: {e}"))
        })?;
        let vout = quote.vout;
        let network = quote.network;

        if let Some(existing) = self
            .store
            .get_by_utxo(&quote.txid, vout)
            .await
            .map_err(StaticDepositError::Store)?
        {
            return match existing.transfer_id {
                Some(_)
                    if existing.is_instant
                        && existing.user_identity_public_key == user_identity_public_key =>
                {
                    Ok(existing.id)
                }
                Some(_) => Err(StaticDepositError::InvalidInput(
                    "the deposit was already claimed".to_string(),
                )),
                None => Err(StaticDepositError::ClaimSettling),
            };
        }

        let output = self.deposit_output(&txid, vout).await?;
        self.check_confirmations(&output)?;
        let fee_rate = self.fee_rate().await?;
        let fee_sats = spend_tx_fee_sats(fee_rate);
        if max_credit_sats(quote.deposit_amount_sats, fee_sats)
            .is_none_or(|max| quote.credit_amount_sats > max)
        {
            return Err(StaticDepositError::InvalidInput(format!(
                "the {fee_sats} sat claim fee has outgrown the quote; re-quote"
            )));
        }

        let encrypted_deposit_secret_key = self
            .resolve_encrypted_deposit_key_from(
                input
                    .encrypted_static_deposit_address_private_key_share
                    .as_deref(),
                input.static_deposit_address_private_key_share.as_deref(),
            )
            .await?;
        // The operators credit the user without checking the key share, and the
        // SSP cannot sign the deposit-spend with the wrong one.
        self.check_deposit_key(
            &encrypted_deposit_secret_key,
            &user_identity_public_key,
            &quote.destination_address,
            network,
        )
        .await?;

        let quote_signature_der = hex::decode(&quote.quote_signature).map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid quote_signature hex: {e}"))
        })?;
        let user_signature_der = hex::decode(&input.signature)
            .map_err(|e| StaticDepositError::InvalidInput(format!("invalid signature hex: {e}")))?;

        let statement = serialize_instant_static_deposit_claim_payload(
            network,
            quote.credit_amount_sats,
            quote.deposit_amount_sats,
            &quote.destination_address,
            &quote_signature_der,
        );
        verify_user_ecdsa(&user_identity_public_key, &user_signature_der, &statement)?;
        let spend_prep = self
            .spend_prep(&txid, vout, quote.deposit_amount_sats, fee_sats)
            .await?;

        let reservation = self.reserve_leaves(quote.credit_amount_sats).await?;
        let credit = PendingCredit {
            transfer_id: TransferId::generate(),
            reservation: HandoverReservation::from(&reservation),
        };
        let now = chrono::Utc::now();
        let record = StaticDepositClaimRecord {
            id: Uuid::now_v7().to_string(),
            user_identity_public_key,
            txid: quote.txid.clone(),
            vout,
            network,
            deposit_address: quote.destination_address.clone(),
            credit_amount_sats: quote.credit_amount_sats,
            is_instant: true,
            deposit_amount_sats: quote.deposit_amount_sats,
            encrypted_deposit_secret_key,
            quote_signature: quote.quote_signature.clone(),
            user_signature: input.signature.clone(),
            transfer_id: None,
            pending_credit: Some(credit.clone()),
            utxo_swap_id: None,
            spend_prep,
            spend_context: None,
            spend_broadcast_txid: None,
            spend_confirmed: false,
            deposit_lost: false,
            created_at: now,
            updated_at: now,
        };
        let call = self.calls.start(&record.id);
        if let Err(e) = self.store.insert(&record).await {
            self.cancel_unless_stored(&record, &reservation).await;
            return Err(StaticDepositError::Store(e));
        }
        let request = match self
            .build_credit_transfer(&record, &credit.transfer_id, &reservation)
            .await
            .and_then(|transfer| reserve_request(&record, transfer))
        {
            Ok(request) => request,
            Err(e) => {
                self.abandon(&record, &reservation).await;
                return Err(e);
            }
        };
        let reserved = self.reserve(&record, &credit, request).await;
        drop(call);
        self.wakeup.wake();
        reserved.map(|()| record.id)
    }

    /// An error from the call does not say whether the operators made the transfer,
    /// so the claim keeps its pending credit for the worker to settle.
    async fn reserve(
        &self,
        record: &StaticDepositClaimRecord,
        credit: &PendingCredit,
        request: crate::operator_rpc::ReserveInstantStaticDepositUtxoSwapRequest,
    ) -> Result<(), StaticDepositError> {
        let response = match crate::operator_rpc::reserve_instant_static_deposit_utxo_swap(
            &self.operator_pool.get_coordinator().client,
            request,
        )
        .await
        {
            Ok(response) => response,
            Err(e) if is_refusal(&e) => return Err(self.refused(record, credit, e).await),
            Err(e) => return Err(e.into()),
        };
        let reservation_id = &credit.reservation.id;
        if let Err(e) = self
            .tree_store
            .finalize_reservation(reservation_id, None)
            .await
        {
            error!(%reservation_id, "failed to finalize reservation: {e:?}");
        }

        let transfer_id = response
            .transfer
            .as_ref()
            .map_or_else(|| credit.transfer_id.to_string(), |t| t.id.clone());
        let utxo_swap_id = response.utxo_swap_id;
        self.store
            .set_reserved(&record.id, &transfer_id, &utxo_swap_id)
            .await
            .map_err(StaticDepositError::Store)?;

        info!(claim_id = %record.id, %transfer_id, %utxo_swap_id, "instant static deposit: reserved, user credited");
        Ok(())
    }

    async fn build_swap_request(
        &self,
        record: &StaticDepositClaimRecord,
        transfer_id: &TransferId,
        reservation: &LeavesReservation,
    ) -> Result<crate::operator_rpc::InitiateStaticDepositUtxoSwapRequest, StaticDepositError> {
        let prep = &record.spend_prep;
        let transfer = self
            .build_credit_transfer(record, transfer_id, reservation)
            .await?;
        let deposit_signing_public_key = self
            .signer
            .public_key_from_secret(&deposit_key(record)?)
            .await?;
        Ok(crate::operator_rpc::InitiateStaticDepositUtxoSwapRequest {
            on_chain_utxo: Some(deposit_utxo(record)?),
            ssp_signature: decode_signature(&record.quote_signature, "quote_signature")?,
            user_signature: decode_signature(&record.user_signature, "signature")?,
            transfer: Some(transfer),
            spend_tx_signing_job: Some(pb::SigningJob {
                signing_public_key: deposit_signing_public_key.serialize().to_vec(),
                raw_tx: bitcoin::consensus::serialize(&prep.spend_tx),
                signing_nonce_commitment: Some(prep.nonce.commitments.try_into()?),
            }),
            ..Default::default()
        })
    }

    async fn spend_prep(
        &self,
        txid: &Txid,
        vout: u32,
        deposit_value: u64,
        fee_sats: u64,
    ) -> Result<StaticDepositSpendPrep, StaticDepositError> {
        let collect_address = self
            .chain
            .new_address()
            .await
            .map_err(|e| StaticDepositError::Chain(e.to_string()))?;
        Ok(StaticDepositSpendPrep {
            spend_tx: build_spend_tx(txid, vout, deposit_value, fee_sats, &collect_address),
            nonce: self.signer.generate_random_signing_commitment().await?,
        })
    }

    /// The transfer has no expiry: an operator returning an expired copy before the
    /// commit reached it would leave the operators disagreeing on the credit, and a
    /// copy returned without an expiry proves the call was rolled back.
    async fn build_credit_transfer(
        &self,
        record: &StaticDepositClaimRecord,
        transfer_id: &TransferId,
        reservation: &LeavesReservation,
    ) -> Result<pb::StartTransferRequest, StaticDepositError> {
        let leaf_key_tweaks = self.build_leaf_key_tweaks(reservation).await?;
        let mut prepared = self
            .transfer_service
            .prepare_transfer_request(
                transfer_id,
                &leaf_key_tweaks,
                &record.user_identity_public_key,
                None,
                None,
                None,
            )
            .await?;
        // The operators do not require direct refunds on this transfer.
        if let Some(package) = prepared.transfer_request.transfer_package.as_mut() {
            package.direct_leaves_to_send.clear();
            package.direct_from_cpfp_leaves_to_send.clear();
        }
        Ok(prepared.transfer_request)
    }

    /// The operators list only UTXOs with the confirmations they require of a deposit.
    async fn operators_list(
        &self,
        deposit_address: &str,
        network: spark::Network,
        txid: &Txid,
        vout: u32,
        exclude_claimed: bool,
    ) -> Result<bool, StaticDepositError> {
        const PAGE: u64 = 100;
        let wanted = hex::decode(txid.to_string())
            .map_err(|e| StaticDepositError::InvalidInput(format!("invalid txid hex: {e}")))?;
        let mut offset = 0;
        loop {
            let page = crate::operator_rpc::get_utxos_for_address(
                &self.operator_pool.get_coordinator().client,
                pb::GetUtxosForAddressRequest {
                    address: deposit_address.to_string(),
                    offset,
                    limit: PAGE,
                    network: network.to_proto_network() as i32,
                    exclude_claimed,
                },
            )
            .await?;
            if page
                .utxos
                .iter()
                .any(|utxo| utxo.txid == wanted && utxo.vout == vout)
            {
                return Ok(true);
            }
            if (page.utxos.len() as u64) < PAGE {
                return Ok(false);
            }
            offset = offset.saturating_add(PAGE);
        }
    }

    async fn claimed_by_another_swap(
        &self,
        record: &StaticDepositClaimRecord,
    ) -> Result<bool, StaticDepositError> {
        let txid = claim_txid(record)?;
        let listed = |exclude_claimed| {
            self.operators_list(
                &record.deposit_address,
                record.network,
                &txid,
                record.vout,
                exclude_claimed,
            )
        };
        Ok(listed(false).await? && !listed(true).await?)
    }

    async fn resolve_encrypted_deposit_key(
        &self,
        input: &ClaimStaticDepositInput,
    ) -> Result<String, StaticDepositError> {
        self.resolve_encrypted_deposit_key_from(
            input.encrypted_deposit_secret_key.as_deref(),
            input.deposit_secret_key.as_deref(),
        )
        .await
    }

    async fn resolve_encrypted_deposit_key_from(
        &self,
        encrypted: Option<&str>,
        cleartext: Option<&str>,
    ) -> Result<String, StaticDepositError> {
        if let Some(encrypted) = encrypted.filter(|s| !s.is_empty()) {
            return Ok(encrypted.to_string());
        }
        let cleartext = cleartext.filter(|s| !s.is_empty()).ok_or_else(|| {
            StaticDepositError::InvalidInput(
                "either the encrypted or cleartext deposit key share is required".to_string(),
            )
        })?;
        let secret_bytes = hex::decode(cleartext).map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid deposit key share hex: {e}"))
        })?;
        let ssp_identity_public_key =
            spark::signer::derive_identity_public_key(self.signer.as_ref()).await?;
        let ciphertext = utils::ecies::encrypt(
            &ssp_identity_public_key.serialize_uncompressed(),
            &secret_bytes,
        )
        .map_err(|e| {
            StaticDepositError::InvalidInput(format!("failed to encrypt deposit key: {e}"))
        })?;
        Ok(hex::encode(ciphertext))
    }

    async fn deposit_output(
        &self,
        txid: &Txid,
        vout: u32,
    ) -> Result<DepositOutput, StaticDepositError> {
        self.chain
            .deposit_output(txid, vout)
            .await
            .map_err(|e| StaticDepositError::Chain(e.to_string()))?
            .ok_or_else(|| {
                StaticDepositError::InvalidInput(format!("{txid}:{vout} is not an unspent output"))
            })
    }

    fn check_confirmations(&self, deposit: &DepositOutput) -> Result<(), StaticDepositError> {
        if deposit.confirmations == 0 && !self.accept_unconfirmed_deposits {
            return Err(StaticDepositError::InvalidInput(
                "the deposit has not confirmed".to_string(),
            ));
        }
        Ok(())
    }

    async fn check_deposit_key(
        &self,
        encrypted_deposit_secret_key: &str,
        user: &PublicKey,
        deposit_address: &str,
        network: spark::Network,
    ) -> Result<(), StaticDepositError> {
        let share = SecretSource::new_encrypted(
            hex::decode(encrypted_deposit_secret_key).map_err(|e| {
                StaticDepositError::InvalidInput(format!("invalid encrypted deposit key hex: {e}"))
            })?,
        );
        let share_public_key = self.signer.public_key_from_secret(&share).await?;
        let addresses = self
            .operator_pool
            .get_coordinator()
            .client
            .query_static_deposit_addresses(pb::QueryStaticDepositAddressesRequest {
                identity_public_key: user.serialize().to_vec(),
                network: network.to_proto_network() as i32,
                limit: 1,
                deposit_address: Some(deposit_address.to_string()),
                ..Default::default()
            })
            .await?;
        let address_key = addresses
            .deposit_addresses
            .first()
            .ok_or_else(|| {
                StaticDepositError::InvalidInput(format!(
                    "{deposit_address} is not one of the user's static deposit addresses"
                ))
            })
            .and_then(|address| {
                PublicKey::from_slice(&address.user_signing_public_key).map_err(|e| {
                    StaticDepositError::InvalidInput(format!(
                        "the operators reported an invalid deposit address key: {e}"
                    ))
                })
            })?;
        if share_public_key != address_key {
            return Err(StaticDepositError::InvalidInput(
                "the deposit key share is not the key of the deposit address".to_string(),
            ));
        }
        Ok(())
    }

    async fn claim_fee(&self) -> Result<u64, StaticDepositError> {
        Ok(claim_fee_sats(self.fee_rate().await?))
    }

    async fn fee_rate(&self) -> Result<u64, StaticDepositError> {
        self.fee_rates
            .sat_per_kw()
            .await
            .map_err(|e| StaticDepositError::Chain(e.to_string()))
    }

    async fn reserve_leaves(&self, amount: u64) -> Result<LeavesReservation, StaticDepositError> {
        let denominations = decompose_into_powers_of_two(amount, self.largest_denomination);
        let target = TargetAmounts::new_exact_denominations(denominations);
        match self
            .tree_store
            .try_reserve_leaves(Some(&target), true, ReservationPurpose::Payment)
            .await?
        {
            ReserveResult::Success(reservation) => Ok(reservation),
            ReserveResult::InsufficientFunds | ReserveResult::WaitForPending { .. } => {
                Err(StaticDepositError::InsufficientPool { amount })
            }
        }
    }

    async fn abandon(&self, record: &StaticDepositClaimRecord, reservation: &LeavesReservation) {
        self.cancel_reservation(reservation).await;
        if let Err(e) = self.store.delete(&record.id).await {
            error!(claim_id = %record.id, "failed to delete an abandoned claim: {e}");
        }
    }

    /// The insert may have committed without its answer arriving; a stored claim
    /// keeps its reservation for the worker to settle.
    async fn cancel_unless_stored(
        &self,
        record: &StaticDepositClaimRecord,
        reservation: &LeavesReservation,
    ) {
        match self.store.get_by_utxo(&record.txid, record.vout).await {
            Ok(Some(stored)) if stored.id == record.id => {}
            Ok(_) => self.cancel_reservation(reservation).await,
            Err(e) => {
                error!(claim_id = %record.id, "could not tell whether a claim was stored, so its leaves stay reserved: {e}");
            }
        }
    }

    async fn cancel_reservation(&self, reservation: &LeavesReservation) {
        if let Err(e) = self
            .tree_store
            .cancel_reservation(&reservation.id, &reservation.leaves)
            .await
        {
            error!(reservation_id = %reservation.id, "failed to cancel reservation: {e:?}");
        }
    }

    async fn build_leaf_key_tweaks(
        &self,
        reservation: &LeavesReservation,
    ) -> Result<Vec<LeafKeyTweak>, StaticDepositError> {
        let mut leaf_key_tweaks = Vec::with_capacity(reservation.leaves.len());
        for node in &reservation.leaves {
            let signing_leaf_id = self
                .key_resolver
                .get_signing_leaf_id(&node.id.to_string())
                .await
                .map_err(|e| StaticDepositError::InvalidInput(format!("key resolver error: {e}")))?
                .map(|id| id.parse::<TreeNodeId>())
                .transpose()
                .map_err(|e| StaticDepositError::InvalidInput(format!("invalid leaf id: {e}")))?
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
}

fn identity_path() -> bitcoin::bip32::DerivationPath {
    bitcoin::bip32::DerivationPath::from(vec![bitcoin::bip32::ChildNumber::Hardened { index: 0 }])
}

fn output_address(output: &TxOut, network: spark::Network) -> Result<String, StaticDepositError> {
    let network: bitcoin::Network = network.into();
    Address::from_script(&output.script_pubkey, network)
        .map(|address| address.to_string())
        .map_err(|e| {
            StaticDepositError::InvalidInput(format!("deposit output is not an address: {e}"))
        })
}

fn claim_txid(record: &StaticDepositClaimRecord) -> Result<Txid, StaticDepositError> {
    Txid::from_str(&record.txid)
        .map_err(|e| StaticDepositError::InvalidInput(format!("invalid claim transaction id: {e}")))
}

/// The operators take the txid bytes in display order and look the UTXO up
/// themselves.
fn deposit_utxo(record: &StaticDepositClaimRecord) -> Result<pb::Utxo, StaticDepositError> {
    Ok(pb::Utxo {
        txid: hex::decode(&record.txid)
            .map_err(|e| StaticDepositError::InvalidInput(format!("invalid txid hex: {e}")))?,
        vout: record.vout,
        network: record.network.to_proto_network() as i32,
        raw_tx: Vec::new(),
    })
}

fn deposit_key(record: &StaticDepositClaimRecord) -> Result<SecretSource, StaticDepositError> {
    Ok(SecretSource::new_encrypted(
        hex::decode(&record.encrypted_deposit_secret_key).map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid encrypted deposit key hex: {e}"))
        })?,
    ))
}

fn decode_signature(signature: &str, field: &str) -> Result<Vec<u8>, StaticDepositError> {
    hex::decode(signature)
        .map_err(|e| StaticDepositError::InvalidInput(format!("invalid {field} hex: {e}")))
}

fn reserve_request(
    record: &StaticDepositClaimRecord,
    transfer: pb::StartTransferRequest,
) -> Result<crate::operator_rpc::ReserveInstantStaticDepositUtxoSwapRequest, StaticDepositError> {
    Ok(
        crate::operator_rpc::ReserveInstantStaticDepositUtxoSwapRequest {
            on_chain_utxo: Some(deposit_utxo(record)?),
            ssp_signature: decode_signature(&record.quote_signature, "quote_signature")?,
            user_signature: decode_signature(&record.user_signature, "signature")?,
            transfer: Some(transfer),
            destination_address: record.deposit_address.clone(),
            value_sats: i64::try_from(record.deposit_amount_sats).map_err(|e| {
                StaticDepositError::InvalidInput(format!("deposit value exceeds i64: {e}"))
            })?,
            credit_amount_sats: i64::try_from(record.credit_amount_sats).map_err(|e| {
                StaticDepositError::InvalidInput(format!("credit amount exceeds i64: {e}"))
            })?,
            secondary_credit_amount_sats: i64::try_from(INSTANT_SECONDARY_CREDIT_AMOUNT_SATS)
                .unwrap_or_default(),
            requested_secondary_transfer_id: String::new(),
        },
    )
}

/// The pre-image of the operators' `CreateStaticDepositSweepStatement` hash. They
/// write the network in capitals.
fn sweep_statement(network: spark::Network, txid: &Txid) -> Vec<u8> {
    let mut statement = b"sweep_static_deposits".to_vec();
    statement.extend_from_slice(network.to_string().to_uppercase().as_bytes());
    statement.extend_from_slice(txid.as_byte_array());
    statement
}

fn p2tr_script(key: &PublicKey) -> ScriptBuf {
    let secp = Secp256k1::new();
    ScriptBuf::new_p2tr(&secp, key.x_only_public_key().0, None)
}

/// The operators co-sign a spend in a claim call only if it is version 3 with lock
/// time 0 and this single input.
fn build_spend_tx(
    txid: &Txid,
    vout: u32,
    deposit_value: u64,
    miner_fee_sats: u64,
    ssp_address: &Address,
) -> Transaction {
    use bitcoin::{TxIn, TxOut, Witness, absolute::LockTime, transaction::Version};

    let output_value = deposit_value
        .saturating_sub(miner_fee_sats)
        .max(fees::P2TR_DUST_SATS)
        .min(deposit_value);
    Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint { txid: *txid, vout },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey: ssp_address.script_pubkey(),
        }],
    }
}

fn verify_user_ecdsa(
    user_identity_public_key: &PublicKey,
    signature_der: &[u8],
    statement: &[u8],
) -> Result<(), StaticDepositError> {
    let signature =
        Signature::from_der(signature_der).map_err(|_| StaticDepositError::InvalidUserSignature)?;
    let digest = sha256::Hash::hash(statement);
    let message = Message::from_digest(digest.to_byte_array());
    Secp256k1::verification_only()
        .verify_ecdsa(&message, &signature, user_identity_public_key)
        .map_err(|_| StaticDepositError::InvalidUserSignature)
}

fn build_spend_context(
    verifying_public_key: Option<&[u8]>,
    signing_result: Option<pb::SigningResult>,
) -> Result<StaticDepositSpendContext, StaticDepositError> {
    let verifying_public_key = verifying_public_key
        .map(PublicKey::from_slice)
        .transpose()
        .map_err(|e| {
            StaticDepositError::InvalidInput(format!("invalid verifying key in the answer: {e}"))
        })?
        .ok_or_else(|| {
            StaticDepositError::InvalidInput(
                "the answer is missing the deposit address verifying key".to_string(),
            )
        })?;
    let signing_result = signing_result.ok_or_else(|| {
        StaticDepositError::InvalidInput(
            "the answer is missing the spend tx signing result".to_string(),
        )
    })?;
    Ok(StaticDepositSpendContext {
        verifying_public_key,
        signing_result,
    })
}

fn deposit_prevout(verifying_public_key: &PublicKey, deposit_value_sats: u64) -> TxOut {
    TxOut {
        value: Amount::from_sat(deposit_value_sats),
        script_pubkey: p2tr_script(verifying_public_key),
    }
}

fn deposit_spend_sighash(
    record: &StaticDepositClaimRecord,
    ctx: &StaticDepositSpendContext,
) -> Result<TapSighash, BoxError> {
    let prevout = deposit_prevout(&ctx.verifying_public_key, record.deposit_amount_sats);
    Ok(sighash_from_tx(&record.spend_prep.spend_tx, 0, &prevout)?)
}

pub struct SparkStaticDepositSpendFinalizer {
    signer: Arc<dyn Signer>,
}

impl SparkStaticDepositSpendFinalizer {
    pub fn new(signer: Arc<dyn Signer>) -> Self {
        Self { signer }
    }
}

#[async_trait::async_trait]
impl StaticDepositSpendFinalizer for SparkStaticDepositSpendFinalizer {
    async fn finalize_spend(
        &self,
        record: &StaticDepositClaimRecord,
    ) -> Result<Transaction, BoxError> {
        let ctx = record.spend_context.as_ref().ok_or_else(|| {
            format!(
                "static deposit claim {} is pending but has no spend context",
                record.id
            )
        })?;

        let sighash = deposit_spend_sighash(record, ctx)?;
        let signing_result: SigningResult = (&ctx.signing_result).try_into()?;
        let nonce = &record.spend_prep.nonce;

        let deposit_source =
            SecretSource::new_encrypted(hex::decode(&record.encrypted_deposit_secret_key)?);
        let deposit_signing_public_key =
            self.signer.public_key_from_secret(&deposit_source).await?;

        let ssp_share = self
            .signer
            .sign_frost(SignFrostRequest {
                message: sighash.as_byte_array(),
                public_key: &ctx.verifying_public_key,
                private_key: &deposit_source,
                verifying_key: &ctx.verifying_public_key,
                self_nonce_commitment: nonce,
                statechain_commitments: signing_result.signing_commitments.clone(),
                adaptor_public_key: None,
            })
            .await?;
        let final_sig = aggregate_frost(AggregateFrostRequest {
            message: sighash.as_byte_array(),
            statechain_signatures: signing_result.signature_shares,
            statechain_public_keys: signing_result.public_keys,
            verifying_key: &ctx.verifying_public_key,
            statechain_commitments: signing_result.signing_commitments,
            self_commitment: &nonce.commitments,
            public_key: &deposit_signing_public_key,
            self_signature: &ssp_share,
            adaptor_public_key: None,
        })?;

        let mut spend_tx = record.spend_prep.spend_tx.clone();
        let mut witness = Witness::new();
        witness.push(
            final_sig
                .serialize()
                .map_err(|e| format!("failed to serialize deposit-spend signature: {e}"))?,
        );
        spend_tx
            .input
            .first_mut()
            .ok_or("the deposit spend has no input")?
            .witness = witness;
        Ok(spend_tx)
    }
}

pub struct SparkStaticDepositSpendCosigner {
    store: Arc<dyn StaticDepositClaimStore>,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    chain: Arc<dyn StaticDepositChain>,
}

impl SparkStaticDepositSpendCosigner {
    pub fn new(
        store: Arc<dyn StaticDepositClaimStore>,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
        chain: Arc<dyn StaticDepositChain>,
    ) -> Self {
        Self {
            store,
            operator_pool,
            signer,
            chain,
        }
    }

    async fn cosign_spend_inner(
        &self,
        record: &StaticDepositClaimRecord,
    ) -> Result<bool, StaticDepositError> {
        let txid = claim_txid(record)?;
        let transfer_id = record.transfer_id.clone().ok_or_else(|| {
            StaticDepositError::InvalidInput("the claim has not credited the user".to_string())
        })?;

        if record.is_instant {
            let Some(deposit) = self
                .chain
                .deposit_output(&txid, record.vout)
                .await
                .map_err(|e| StaticDepositError::Chain(e.to_string()))?
            else {
                self.deposit_missing(record).await?;
                return Ok(false);
            };
            if deposit.confirmations < INSTANT_CLAIM_MIN_CONFIRMATIONS {
                return Ok(false);
            }
        }

        let deposit_signing_public_key = self
            .signer
            .public_key_from_secret(&deposit_key(record)?)
            .await?;
        let StaticDepositSpendPrep { spend_tx, nonce } = &record.spend_prep;
        let (verifying_public_key, signing_result) = if record.is_instant {
            self.claim(record, spend_tx, nonce, &deposit_signing_public_key)
                .await?
        } else {
            self.sweep(record, spend_tx, nonce).await?
        };

        let context = build_spend_context(verifying_public_key.as_deref(), signing_result)?;
        self.store
            .set_spend_context(&record.id, &transfer_id, &context)
            .await
            .map_err(StaticDepositError::Store)?;

        info!(claim_id = %record.id, "static deposit: deposit-spend co-signed");
        Ok(true)
    }

    /// The user can replace an INSTANT deposit before it confirms, which the SSP
    /// accepts as a loss.
    async fn deposit_missing(
        &self,
        record: &StaticDepositClaimRecord,
    ) -> Result<(), StaticDepositError> {
        let given_up = record
            .created_at
            .checked_add_signed(DEPOSIT_LOST_AFTER)
            .is_none_or(|time| time < chrono::Utc::now());
        if !given_up {
            warn!(claim_id = %record.id, txid = %record.txid, vout = record.vout, "static deposit: the deposit of a credited INSTANT claim is missing");
            return Ok(());
        }
        self.store
            .set_deposit_lost(&record.id)
            .await
            .map_err(StaticDepositError::Store)?;
        error!(claim_id = %record.id, txid = %record.txid, vout = record.vout, credit = record.credit_amount_sats, "static deposit: gave up a credited INSTANT claim whose deposit vanished");
        Ok(())
    }

    async fn claim(
        &self,
        record: &StaticDepositClaimRecord,
        spend_tx: &Transaction,
        nonce: &FrostSigningCommitmentsWithNonces,
        deposit_signing_public_key: &PublicKey,
    ) -> Result<(Option<Vec<u8>>, Option<pb::SigningResult>), StaticDepositError> {
        let utxo_swap_id = record.utxo_swap_id.clone().ok_or_else(|| {
            StaticDepositError::InvalidInput(
                "instant claim record is not reserved (no utxo swap id)".to_string(),
            )
        })?;
        let request = crate::operator_rpc::ClaimInstantStaticDepositUtxoSwapRequest {
            on_chain_utxo: Some(deposit_utxo(record)?),
            utxo_swap_id,
            transfer: None,
            spend_tx_signing_job: Some(pb::SigningJob {
                signing_public_key: deposit_signing_public_key.serialize().to_vec(),
                raw_tx: bitcoin::consensus::serialize(spend_tx),
                signing_nonce_commitment: Some(nonce.commitments.try_into()?),
            }),
        };
        let response = crate::operator_rpc::claim_instant_static_deposit_utxo_swap(
            &self.operator_pool.get_coordinator().client,
            request,
        )
        .await?;
        Ok((
            response
                .deposit_address
                .map(|address| address.verifying_public_key),
            response.spend_tx_signing_result,
        ))
    }

    async fn sweep(
        &self,
        record: &StaticDepositClaimRecord,
        spend_tx: &Transaction,
        nonce: &FrostSigningCommitmentsWithNonces,
    ) -> Result<(Option<Vec<u8>>, Option<pb::SigningResult>), StaticDepositError> {
        use crate::operator_rpc::sign_static_deposit_sweep_tx_response::Result as SweepResult;

        let statement = sweep_statement(record.network, &spend_tx.compute_txid());
        let ssp_signature = self
            .signer
            .sign_message_ecdsa(&identity_path(), &statement)
            .await?;
        let request = crate::operator_rpc::SignStaticDepositSweepTxRequest {
            network: record.network.to_proto_network() as i32,
            raw_tx: bitcoin::consensus::serialize(spend_tx),
            inputs: vec![crate::operator_rpc::SweepInput {
                on_chain_utxo: Some(deposit_utxo(record)?),
                vin: 0,
                user_signing_commitment: Some(nonce.commitments.try_into()?),
            }],
            ssp_signature: ssp_signature.serialize_der().to_vec(),
        };
        let response = crate::operator_rpc::sign_static_deposit_sweep_tx(
            &self.operator_pool.get_coordinator().client,
            request,
        )
        .await?;
        match response.result {
            Some(SweepResult::Signed(signed)) => {
                let input = signed
                    .results
                    .into_iter()
                    .find(|result| result.vin == 0)
                    .ok_or_else(|| {
                        StaticDepositError::InvalidInput(
                            "the sweep answer has no signing result for its input".to_string(),
                        )
                    })?;
                Ok((Some(input.verifying_key), input.signing_result))
            }
            Some(SweepResult::Ineligible(ineligible)) => Err(StaticDepositError::InvalidInput(
                format!("the operators refuse to sweep the deposit: {ineligible:?}"),
            )),
            None => Err(StaticDepositError::InvalidInput(
                "the sweep answer is empty".to_string(),
            )),
        }
    }
}

#[async_trait::async_trait]
impl StaticDepositSpendCosigner for SparkStaticDepositSpendCosigner {
    async fn cosign_spend(&self, record: &StaticDepositClaimRecord) -> Result<bool, BoxError> {
        Ok(self.cosign_spend_inner(record).await?)
    }
}

pub struct StaticDepositWorkerDeps {
    pub store: Arc<dyn StaticDepositClaimStore>,
    pub quotes: Arc<dyn InstantStaticDepositQuoteStore>,
    pub chain: Arc<dyn StaticDepositChain>,
    pub finalizer: Arc<dyn StaticDepositSpendFinalizer>,
    pub spend_cosigner: Arc<dyn StaticDepositSpendCosigner>,
    pub credit_settler: Arc<dyn PendingCreditSettler>,
    pub blocks: Wakeup,
    pub claims: Wakeup,
}

pub async fn run_static_deposit_loop(deps: StaticDepositWorkerDeps, token: CancellationToken) {
    info!("Starting static deposit loop");
    loop {
        let new_block = tokio::select! {
            () = token.cancelled() => {
                info!("Static deposit loop cancelled");
                return;
            }
            () = deps.blocks.waited() => true,
            () = deps.claims.waited() => false,
            () = tokio::time::sleep(STATIC_DEPOSIT_BACKUP_INTERVAL) => false,
        };
        if let Err(e) = process_pending_static_deposit_claims(&deps, new_block).await {
            error!("Static deposit check failed: {e}");
        }
    }
}

/// A spend already broadcast is broadcast again, and checked for confirmation, once
/// per block, since a node can drop an unconfirmed transaction.
pub async fn process_pending_static_deposit_claims(
    deps: &StaticDepositWorkerDeps,
    new_block: bool,
) -> Result<(), BoxError> {
    if new_block
        && let Some(expired) = chrono::Utc::now().checked_sub_signed(INSTANT_QUOTE_LIFETIME)
    {
        deps.quotes.delete_created_before(expired).await?;
    }
    for record in deps.store.pending().await? {
        if let Err(e) = process_static_deposit_claim(deps, &record, new_block).await {
            error!(claim_id = %record.id, "failed to advance static deposit claim: {e}");
        }
    }
    Ok(())
}

async fn process_static_deposit_claim(
    deps: &StaticDepositWorkerDeps,
    record: &StaticDepositClaimRecord,
    new_block: bool,
) -> Result<(), BoxError> {
    if let Some(credit) = &record.pending_credit {
        return deps.credit_settler.settle(record, credit).await;
    }

    let record = if record.spend_context.is_none() {
        if !deps.spend_cosigner.cosign_spend(record).await? {
            return Ok(());
        }
        match deps.store.get(&record.id).await? {
            Some(updated) => updated,
            None => return Ok(()),
        }
    } else {
        record.clone()
    };

    if record.spend_broadcast_txid.is_some() && !new_block {
        return Ok(());
    }
    let spend_tx = deps.finalizer.finalize_spend(&record).await?;
    if deps.chain.is_confirmed(&spend_tx).await? {
        deps.store.set_spend_confirmed(&record.id).await?;
        info!(claim_id = %record.id, "static deposit: deposit-spend confirmed");
        return Ok(());
    }
    deps.chain.broadcast(&spend_tx).await?;
    if record.spend_broadcast_txid.is_none() {
        let txid = spend_tx.compute_txid();
        deps.store
            .set_spend_broadcast_txid(&record.id, &txid.to_string())
            .await?;
        info!(claim_id = %record.id, %txid, "static deposit: deposit-spend broadcast");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pb;
    use super::repository::{
        InMemoryInstantStaticDepositQuoteStore, InMemoryStaticDepositClaimStore,
        StaticDepositClaimRecord, StaticDepositClaimStore, StaticDepositSpendContext,
        StaticDepositSpendPrep,
    };
    use super::{
        BoxError, ClaimCalls, DepositOutput, HandoverReservation, PendingCredit,
        PendingCreditSettler, SparkStaticDepositSpendFinalizer, StaticDepositChain,
        StaticDepositSpendCosigner, StaticDepositSpendFinalizer, StaticDepositWorkerDeps,
        TransferId, build_spend_tx, deposit_prevout, deposit_spend_sighash, max_credit_sats,
        p2tr_script, process_pending_static_deposit_claims, sweep_statement,
    };
    use bitcoin::hashes::Hash as _;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use bitcoin::{Address, Amount, Transaction, TxOut, Txid};
    use spark::bitcoin::sighash_from_tx;
    use spark::signer::{DefaultSigner, FrostSigningCommitmentsWithNonces, Signer};
    use std::str::FromStr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn pubkey(byte: u8) -> PublicKey {
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).expect("secret"))
    }

    fn address(byte: u8) -> Address {
        Address::from_script(&p2tr_script(&pubkey(byte)), bitcoin::Network::Regtest)
            .expect("address")
    }

    fn deposit_txid() -> Txid {
        Txid::from_str("0000000000000000000000000000000000000000000000000000000000000001").unwrap()
    }

    #[test]
    fn spend_tx_weight_matches_the_built_tx() {
        let tx = build_spend_tx(&Txid::all_zeros(), 0, 10_000, 300, &address(1));
        // Signing adds the segwit marker and flag, and one key-path signature.
        let signed = tx.weight().to_wu() + 2 + crate::fees::P2TR_KEY_PATH_WITNESS_WU;
        assert_eq!(signed, super::SPEND_TX_WEIGHT_WU);
    }

    #[test]
    fn the_collected_output_keeps_the_dust_limit() {
        let dust = crate::fees::P2TR_DUST_SATS;
        let output = |deposit, fee| {
            build_spend_tx(&deposit_txid(), 0, deposit, fee, &address(1)).output[0]
                .value
                .to_sat()
        };
        assert_eq!(output(10_000, 300), 9_700);
        assert_eq!(output(dust + 100, 300), dust);
    }

    #[test]
    fn a_credit_leaves_the_claim_fee_and_dust() {
        let dust = crate::fees::P2TR_DUST_SATS;
        assert_eq!(max_credit_sats(10_000, 500), Some(9_500));
        assert_eq!(max_credit_sats(500 + dust, 500), Some(dust));
        assert_eq!(max_credit_sats(500 + dust - 1, 500), None);
        assert_eq!(max_credit_sats(400, 500), None);
    }

    fn test_signer() -> DefaultSigner {
        DefaultSigner::new(&[7u8; 32], spark::Network::Regtest).expect("signer")
    }

    async fn real_nonce() -> FrostSigningCommitmentsWithNonces {
        test_signer()
            .generate_random_signing_commitment()
            .await
            .expect("nonce")
    }

    fn spend_context() -> StaticDepositSpendContext {
        StaticDepositSpendContext {
            verifying_public_key: pubkey(9),
            signing_result: pb::SigningResult::default(),
        }
    }

    #[derive(Default)]
    struct StubChain {
        confirmed: AtomicBool,
        broadcasts: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl StaticDepositChain for StubChain {
        async fn deposit_output(
            &self,
            _txid: &Txid,
            _vout: u32,
        ) -> Result<Option<DepositOutput>, BoxError> {
            Ok(None)
        }

        async fn new_address(&self) -> Result<Address, BoxError> {
            Ok(address(3))
        }

        async fn is_confirmed(&self, _tx: &Transaction) -> Result<bool, BoxError> {
            Ok(self.confirmed.load(Ordering::SeqCst))
        }

        async fn broadcast(&self, _tx: &Transaction) -> Result<(), BoxError> {
            self.broadcasts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct StubFinalizer;

    #[async_trait::async_trait]
    impl StaticDepositSpendFinalizer for StubFinalizer {
        async fn finalize_spend(
            &self,
            record: &StaticDepositClaimRecord,
        ) -> Result<Transaction, BoxError> {
            record.spend_context.as_ref().ok_or("no spend context")?;
            Ok(record.spend_prep.spend_tx.clone())
        }
    }

    async fn record(id: &str) -> StaticDepositClaimRecord {
        StaticDepositClaimRecord {
            id: id.to_string(),
            user_identity_public_key: pubkey(1),
            txid: deposit_txid().to_string(),
            vout: 0,
            network: spark::Network::Regtest,
            deposit_address: "bcrt1pdeposit".to_string(),
            credit_amount_sats: 10_000,
            is_instant: false,
            deposit_amount_sats: 50_000,
            encrypted_deposit_secret_key: "deadbeef".to_string(),
            quote_signature: "3045".to_string(),
            user_signature: "3044".to_string(),
            transfer_id: None,
            pending_credit: None,
            utxo_swap_id: None,
            spend_prep: StaticDepositSpendPrep {
                spend_tx: build_spend_tx(&deposit_txid(), 0, 50_000, 300, &address(3)),
                nonce: real_nonce().await,
            },
            spend_context: None,
            spend_broadcast_txid: None,
            spend_confirmed: false,
            deposit_lost: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[derive(Default)]
    struct StubCreditSettler {
        settled: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl PendingCreditSettler for StubCreditSettler {
        async fn settle(
            &self,
            _record: &StaticDepositClaimRecord,
            _credit: &PendingCredit,
        ) -> Result<(), BoxError> {
            self.settled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn deps(
        store: &Arc<InMemoryStaticDepositClaimStore>,
        chain: &Arc<StubChain>,
        cosigner: Arc<dyn StaticDepositSpendCosigner>,
    ) -> StaticDepositWorkerDeps {
        StaticDepositWorkerDeps {
            store: Arc::clone(store) as Arc<dyn StaticDepositClaimStore>,
            quotes: Arc::new(InMemoryInstantStaticDepositQuoteStore::default()),
            chain: Arc::clone(chain) as Arc<dyn StaticDepositChain>,
            finalizer: Arc::new(StubFinalizer),
            spend_cosigner: cosigner,
            credit_settler: Arc::new(StubCreditSettler::default()),
            blocks: crate::wakeup::Wakeup::new(),
            claims: crate::wakeup::Wakeup::new(),
        }
    }

    #[tokio::test]
    async fn worker_hands_every_pending_credit_to_the_settler() {
        let store = Arc::new(InMemoryStaticDepositClaimStore::default());
        let chain = Arc::new(StubChain::default());
        let settler = Arc::new(StubCreditSettler::default());
        let deps = StaticDepositWorkerDeps {
            credit_settler: Arc::clone(&settler) as Arc<dyn PendingCreditSettler>,
            ..deps(
                &store,
                &chain,
                Arc::new(StubCosigner::confirmed(&store, false)),
            )
        };
        let credit = PendingCredit {
            transfer_id: TransferId::generate(),
            reservation: HandoverReservation {
                id: "reservation".to_string(),
                leaf_ids: Vec::new(),
            },
        };
        for id in ["a", "b"] {
            let mut pending = record(id).await;
            pending.pending_credit = Some(credit.clone());
            store.insert(&pending).await.unwrap();
        }

        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        assert_eq!(settler.settled.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_claim_call_is_running_until_its_guard_drops() {
        let calls = ClaimCalls::default();
        let call = calls.start("claim").expect("nothing is running");
        assert!(calls.start("claim").is_none());
        assert!(calls.start("other").is_some());
        drop(call);
        assert!(calls.start("claim").is_some());
    }

    #[test]
    fn sweep_statement_matches_the_operators() {
        let txid = deposit_txid();
        let mut expected = b"sweep_static_deposits".to_vec();
        expected.extend_from_slice(b"REGTEST");
        expected.extend_from_slice(&txid.to_byte_array());
        assert_eq!(sweep_statement(spark::Network::Regtest, &txid), expected);
    }

    #[tokio::test]
    async fn worker_broadcasts_the_spend_until_it_confirms() {
        let store = Arc::new(InMemoryStaticDepositClaimStore::default());
        let chain = Arc::new(StubChain::default());
        let cosigner = Arc::new(StubCosigner::confirmed(&store, false));
        let deps = deps(
            &store,
            &chain,
            Arc::clone(&cosigner) as Arc<dyn StaticDepositSpendCosigner>,
        );

        store.insert(&record("s1").await).await.unwrap();
        store.set_transfer_id("s1", "transfer-1").await.unwrap();
        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        assert_eq!(cosigner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(chain.broadcasts.load(Ordering::SeqCst), 0);

        store
            .set_spend_context("s1", "transfer-1", &spend_context())
            .await
            .unwrap();
        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        assert_eq!(chain.broadcasts.load(Ordering::SeqCst), 1);
        process_pending_static_deposit_claims(&deps, true)
            .await
            .unwrap();
        assert_eq!(chain.broadcasts.load(Ordering::SeqCst), 2);
        let broadcast = store.get("s1").await.unwrap().unwrap();
        assert!(broadcast.spend_broadcast_txid.is_some());
        assert!(!broadcast.spend_confirmed);

        chain.confirmed.store(true, Ordering::SeqCst);
        process_pending_static_deposit_claims(&deps, true)
            .await
            .unwrap();
        assert!(store.get("s1").await.unwrap().unwrap().spend_confirmed);
        assert!(store.pending().await.unwrap().is_empty());
        assert_eq!(chain.broadcasts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn deposit_spend_sighash_matches_reconstructed_prevout() {
        let verifying_key = pubkey(9);
        let rec = record("s1").await;
        let deposit_value = rec.deposit_amount_sats;
        let spend_tx = rec.spend_prep.spend_tx.clone();
        let ctx = spend_context();

        let expected_prevout = TxOut {
            value: Amount::from_sat(deposit_value),
            script_pubkey: p2tr_script(&verifying_key),
        };
        assert_eq!(
            deposit_prevout(&verifying_key, deposit_value),
            expected_prevout
        );
        let expected = sighash_from_tx(&spend_tx, 0, &expected_prevout).unwrap();
        assert_eq!(deposit_spend_sighash(&rec, &ctx).unwrap(), expected);
    }

    #[tokio::test]
    async fn finalize_errors_without_spend_context() {
        let signer = Arc::new(test_signer()) as Arc<dyn Signer>;
        let finalizer = SparkStaticDepositSpendFinalizer::new(signer);

        let mut rec = record("s1").await;
        rec.transfer_id = Some("t1".to_string());
        let err = finalizer.finalize_spend(&rec).await.unwrap_err();
        assert!(err.to_string().contains("no spend context"));
    }

    struct StubCosigner {
        store: Arc<InMemoryStaticDepositClaimStore>,
        confirmed: AtomicBool,
        calls: AtomicUsize,
    }

    impl StubCosigner {
        fn confirmed(store: &Arc<InMemoryStaticDepositClaimStore>, confirmed: bool) -> Self {
            Self {
                store: Arc::clone(store),
                confirmed: AtomicBool::new(confirmed),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl StaticDepositSpendCosigner for StubCosigner {
        async fn cosign_spend(&self, record: &StaticDepositClaimRecord) -> Result<bool, BoxError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.confirmed.load(Ordering::SeqCst) {
                return Ok(false);
            }
            let transfer_id = record.transfer_id.clone().unwrap();
            self.store
                .set_spend_context(&record.id, &transfer_id, &spend_context())
                .await?;
            Ok(true)
        }
    }

    #[tokio::test]
    async fn worker_runs_deferred_claim_before_finalizing() {
        let store = Arc::new(InMemoryStaticDepositClaimStore::default());
        let chain = Arc::new(StubChain::default());
        let claimer = Arc::new(StubCosigner::confirmed(&store, false));
        let deps = deps(
            &store,
            &chain,
            Arc::clone(&claimer) as Arc<dyn StaticDepositSpendCosigner>,
        );

        let mut rec = record("i1").await;
        rec.is_instant = true;
        store.insert(&rec).await.unwrap();
        store
            .set_reserved("i1", "transfer-i1", "swap-i1")
            .await
            .unwrap();

        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        assert_eq!(claimer.calls.load(Ordering::SeqCst), 1);
        assert_eq!(chain.broadcasts.load(Ordering::SeqCst), 0);

        claimer.confirmed.store(true, Ordering::SeqCst);
        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        assert_eq!(claimer.calls.load(Ordering::SeqCst), 2);
        assert_eq!(chain.broadcasts.load(Ordering::SeqCst), 1);
        assert!(
            store
                .get("i1")
                .await
                .unwrap()
                .unwrap()
                .spend_broadcast_txid
                .is_some()
        );

        process_pending_static_deposit_claims(&deps, false)
            .await
            .unwrap();
        assert_eq!(claimer.calls.load(Ordering::SeqCst), 2);
    }
}
