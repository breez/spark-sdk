//! Cooperative exits: the SSP pays a user on chain for the leaves the user
//! transfers to it, broadcasting only once that transfer is verified.

pub mod repository;

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::fees::{self, FeeRateSource};
use crate::wallet::onchain::{HeldCoins, Utxo};
use bitcoin::secp256k1::PublicKey;
use spark::operator::OperatorPool;
use spark::services::{
    ServiceError, Transfer, TransferId, TransferService, TransferStatus, TransferType,
};
use spark::signer::{Signer, SignerError};
use spark::tree::TreeStore;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::graphql::types::RequestCoopExitInput;
use crate::leaves::{IncomingTransfer, LeafSigningKeys, claim_into_pool};
use crate::wakeup::Wakeup;

use self::repository::{CoopExitPrevout, CoopExitRecord, CoopExitStore};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The worker also wakes on every completed request and every block.
const COOP_EXIT_BACKUP_INTERVAL: Duration = Duration::from_secs(60);

/// Nodes do not relay a transaction weighing more.
const MAX_STANDARD_TX_WEIGHT_WU: u64 = 400_000;

/// An exit is rebroadcast until it has this many confirmations, even once its
/// leaves are claimed: a reorg can unconfirm it.
const EXIT_SETTLED_CONFIRMATIONS: u64 = 6;

pub const INCOMPLETE_REQUEST_TTL: Duration = Duration::from_secs(60 * 60);

/// A quoted fee is sized before coin selection runs, so it assumes this many
/// wallet inputs.
const QUOTED_COOP_EXIT_INPUTS: u64 = 2;

/// Headroom on a quoted fee for a coin selection needing more inputs than assumed.
const QUOTE_FEE_MARGIN_PERCENT: u64 = 20;

const REFUND_TX_WEIGHT_WU: u64 =
    fees::TX_OVERHEAD_WU + 2 * fees::P2TR_INPUT_WU + fees::P2TR_OUTPUT_WU + fees::P2A_OUTPUT_WU;

fn connector_tx_weight_wu(num_leaves: u64) -> u64 {
    fees::TX_OVERHEAD_WU
        .saturating_add(fees::P2TR_INPUT_WU)
        .saturating_add(
            num_leaves
                .saturating_add(1)
                .saturating_mul(fees::P2TR_OUTPUT_WU),
        )
}

/// Nodes relay a version 3 transaction of at most 10,000 vbytes, and the
/// connector transaction has an output for each leaf.
pub const MAX_COOP_EXIT_LEAVES: u64 =
    (40_000 - fees::TX_OVERHEAD_WU - fees::P2TR_INPUT_WU) / fees::P2TR_OUTPUT_WU - 1;

pub fn check_leaf_count(num_leaves: u64) -> Result<(), CoopExitError> {
    if num_leaves > MAX_COOP_EXIT_LEAVES {
        return Err(CoopExitError::InvalidInput(format!(
            "an exit carries at most {MAX_COOP_EXIT_LEAVES} leaves, not {num_leaves}"
        )));
    }
    Ok(())
}

fn coop_exit_tx_weight_wu(num_inputs: u64, num_outputs: u64) -> u64 {
    fees::TX_OVERHEAD_WU
        .saturating_add(num_inputs.saturating_mul(fees::P2TR_INPUT_WU))
        .saturating_add(num_outputs.saturating_mul(fees::P2TR_OUTPUT_WU))
}

/// A leaf's refund spends its connector output, and the operators fix the refund's
/// outputs, so the connector's value goes to the refund's fee.
fn per_leaf_connector_sats(sat_per_kw: u64) -> u64 {
    fees::fee_sats(sat_per_kw, REFUND_TX_WEIGHT_WU).max(fees::P2TR_DUST_SATS)
}

fn connector_funding_sats(num_leaves: u64, sat_per_kw: u64) -> u64 {
    num_leaves
        .saturating_mul(per_leaf_connector_sats(sat_per_kw))
        .saturating_add(fees::P2TR_DUST_SATS)
        .saturating_add(fees::fee_sats(
            sat_per_kw,
            connector_tx_weight_wu(num_leaves),
        ))
}

fn miner_fees_sats(
    spent: u64,
    exit_tx: &bitcoin::Transaction,
    connector_tx: &bitcoin::Transaction,
) -> u64 {
    let exit_outputs: u64 = exit_tx.output.iter().map(|o| o.value.to_sat()).sum();
    let connector_funding = exit_tx.output.get(1).map_or(0, |o| o.value.to_sat());
    let connector_outputs: u64 = connector_tx.output.iter().map(|o| o.value.to_sat()).sum();
    spent
        .saturating_sub(exit_outputs)
        .saturating_add(connector_funding.saturating_sub(connector_outputs))
}

fn with_margin(sats: u64) -> u64 {
    sats.saturating_add(
        sats.saturating_mul(QUOTE_FEE_MARGIN_PERCENT)
            .saturating_div(100),
    )
}

pub struct CoopExitFees {
    pub l1_broadcast_fee_sats: u64,
    pub user_fee_sats: u64,
}

#[must_use]
pub fn coop_exit_fees(num_leaves: u64, sat_per_kw: u64) -> CoopExitFees {
    let exit_tx_fee = fees::fee_sats(
        sat_per_kw,
        coop_exit_tx_weight_wu(QUOTED_COOP_EXIT_INPUTS, 3),
    );
    CoopExitFees {
        l1_broadcast_fee_sats: with_margin(
            connector_funding_sats(num_leaves, sat_per_kw).saturating_add(exit_tx_fee),
        ),
        user_fee_sats: 0,
    }
}

#[async_trait::async_trait]
pub trait CoopExitOnchainWallet: Send + Sync {
    fn network(&self) -> bitcoin::Network;
    /// Takes coins covering `need(inputs)`, held from every other selection
    /// until the returned [`HeldCoins`] is dropped.
    async fn select_coins(
        &self,
        need: &(dyn Fn(usize) -> u64 + Send + Sync),
    ) -> Result<(Vec<Utxo>, HeldCoins), BoxError>;
    /// A fresh taproot address the wallet can sign for.
    async fn next_address(&self) -> Result<bitcoin::Address, BoxError>;
    /// The untweaked secret key of one of this wallet's taproot addresses.
    async fn derive_keypair_for_address(
        &self,
        address: &bitcoin::Address,
    ) -> Result<bitcoin::secp256k1::SecretKey, BoxError>;
}

#[derive(Debug, thiserror::Error)]
pub enum CoopExitError {
    #[error("spark service error: {0}")]
    Service(#[from] ServiceError),
    #[error("signer error: {0}")]
    Signer(#[from] SignerError),
    #[error("storage error: {0}")]
    Store(String),
    #[error("wallet error: {0}")]
    Wallet(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("transfer not found: {0}")]
    TransferNotFound(String),
    #[error("coop-exit request not found: {0}")]
    NotFound(String),
}

pub struct BuiltCoopExitTxs {
    pub raw_coop_exit_tx: Vec<u8>,
    pub raw_connector_tx: Vec<u8>,
    pub coop_exit_txid: String,
    pub amount_sats: u64,
    pub fee_sats: u64,
    pub prevouts: Vec<CoopExitPrevout>,
}

pub struct CoopExitService {
    store: Arc<dyn CoopExitStore>,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    transfer_service: Arc<TransferService>,
    wallet: Arc<dyn CoopExitOnchainWallet>,
    fee_rates: Arc<dyn FeeRateSource>,
    network: spark::Network,
    worker: Wakeup,
}

impl CoopExitService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<dyn CoopExitStore>,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
        transfer_service: Arc<TransferService>,
        wallet: Arc<dyn CoopExitOnchainWallet>,
        fee_rates: Arc<dyn FeeRateSource>,
        network: spark::Network,
        worker: Wakeup,
    ) -> Self {
        Self {
            store,
            operator_pool,
            signer,
            transfer_service,
            wallet,
            fee_rates,
            network,
            worker,
        }
    }

    pub fn network(&self) -> bitcoin::Network {
        self.wallet.network()
    }

    /// The user's transfer is not verified here: the client makes it only once it
    /// has this request's transactions.
    pub async fn request_coop_exit(
        &self,
        input: &RequestCoopExitInput,
        user_identity_public_key: PublicKey,
    ) -> Result<CoopExitRecord, CoopExitError> {
        let transfer_uuid = input.user_outbound_transfer_external_id.ok_or_else(|| {
            CoopExitError::InvalidInput(
                "user_outbound_transfer_external_id is required".to_string(),
            )
        })?;
        let transfer_id = TransferId::from_str(&transfer_uuid.to_string())
            .map_err(|e| CoopExitError::InvalidInput(format!("invalid transfer id: {e}")))?;

        if let Some(existing) = self
            .store
            .get_by_transfer_id(&transfer_id)
            .await
            .map_err(CoopExitError::Store)?
        {
            if existing.user_identity_public_key == user_identity_public_key {
                return Ok(existing);
            }
            return Err(CoopExitError::InvalidInput(
                "transfer already backs another coop exit".to_string(),
            ));
        }

        let fee_leaf_external_ids = input.fee_leaf_external_ids.clone().unwrap_or_default();
        let leaf_ids: Vec<String> = input
            .leaf_external_ids
            .iter()
            .chain(&fee_leaf_external_ids)
            .map(ToString::to_string)
            .collect();
        if leaf_ids.iter().collect::<HashSet<_>>().len() != leaf_ids.len() {
            return Err(CoopExitError::InvalidInput(
                "a leaf is named more than once".to_string(),
            ));
        }
        if self
            .store
            .any_leaf_in_open_request(&leaf_ids)
            .await
            .map_err(CoopExitError::Store)?
        {
            return Err(CoopExitError::InvalidInput(
                "a leaf already backs another open exit request".to_string(),
            ));
        }
        let (built, held) = self
            .build_coop_exit_txs(
                &input.leaf_external_ids,
                &fee_leaf_external_ids,
                &input.withdrawal_address,
                input.withdraw_all,
                &user_identity_public_key,
            )
            .await?;

        let now = chrono::Utc::now();
        let record = CoopExitRecord {
            id: Uuid::now_v7().to_string(),
            user_identity_public_key,
            user_transfer_id: transfer_id,
            withdrawal_address: input.withdrawal_address.clone(),
            amount_sats: built.amount_sats,
            fee_sats: built.fee_sats,
            raw_coop_exit_tx: built.raw_coop_exit_tx,
            raw_connector_tx: built.raw_connector_tx,
            coop_exit_txid: built.coop_exit_txid,
            prevouts: built.prevouts,
            completed: false,
            broadcast_txid: None,
            leaves_claimed: false,
            settled: false,
            created_at: now,
            updated_at: now,
        };
        self.store
            .insert(&record, &leaf_ids)
            .await
            .map_err(CoopExitError::Store)?;
        drop(held);
        Ok(record)
    }

    pub async fn complete_coop_exit(
        &self,
        request_id: &str,
        caller: PublicKey,
    ) -> Result<CoopExitRecord, CoopExitError> {
        let record = self
            .store
            .get(request_id)
            .await
            .map_err(CoopExitError::Store)?
            .filter(|record| record.user_identity_public_key == caller)
            .ok_or_else(|| CoopExitError::NotFound(request_id.to_string()))?;

        self.verify_committed(&record).await?;

        self.store
            .set_completed(request_id)
            .await
            .map_err(CoopExitError::Store)?;
        self.worker.wake();
        self.store
            .get(request_id)
            .await
            .map_err(CoopExitError::Store)?
            .ok_or_else(|| CoopExitError::NotFound(request_id.to_string()))
    }

    pub async fn request_for(
        &self,
        id: &str,
        caller: &PublicKey,
    ) -> Result<Option<CoopExitRecord>, CoopExitError> {
        Ok(self
            .store
            .get(id)
            .await
            .map_err(CoopExitError::Store)?
            .filter(|record| record.user_identity_public_key == *caller))
    }

    pub async fn request_for_transfer(
        &self,
        transfer_id: &TransferId,
        caller: &PublicKey,
    ) -> Result<Option<CoopExitRecord>, CoopExitError> {
        Ok(self
            .store
            .get_by_transfer_id(transfer_id)
            .await
            .map_err(CoopExitError::Store)?
            .filter(|record| record.user_identity_public_key == *caller))
    }

    async fn fee_rate(&self) -> Result<u64, CoopExitError> {
        self.fee_rates
            .sat_per_kw()
            .await
            .map_err(|e| CoopExitError::Wallet(format!("fee rate unavailable: {e}")))
    }

    /// The operators require the connector tx to be version 3 with a single input
    /// spending the exit tx, an empty scriptSig and witness, and one output for each
    /// leaf of the user's transfer, fee leaves included, plus one more.
    #[allow(clippy::too_many_lines)]
    async fn build_coop_exit_txs(
        &self,
        leaf_external_ids: &[Uuid],
        fee_leaf_external_ids: &[Uuid],
        withdrawal_address: &str,
        withdraw_all: bool,
        owner: &PublicKey,
    ) -> Result<(BuiltCoopExitTxs, HeldCoins), CoopExitError> {
        use bitcoin::{
            Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
            absolute::LockTime, address::NetworkUnchecked, transaction::Version,
        };

        if leaf_external_ids.is_empty() {
            return Err(CoopExitError::InvalidInput(
                "no leaves specified".to_string(),
            ));
        }
        let num_amount_leaves = leaf_external_ids.len() as u64;
        let num_connector_leaves =
            num_amount_leaves.saturating_add(fee_leaf_external_ids.len() as u64);
        check_leaf_count(num_connector_leaves)?;
        let values = self
            .owned_leaf_values(
                &leaf_external_ids
                    .iter()
                    .chain(fee_leaf_external_ids)
                    .copied()
                    .collect::<Vec<_>>(),
                owner,
            )
            .await?;
        let leaves_sum: u64 = leaf_external_ids
            .iter()
            .filter_map(|id| values.get(id))
            .sum();

        let network = self.wallet.network();
        let withdrawal = withdrawal_address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|e| CoopExitError::InvalidInput(format!("invalid withdrawal address: {e}")))?
            .require_network(network)
            .map_err(|e| {
                CoopExitError::InvalidInput(format!("withdrawal address wrong network: {e}"))
            })?;

        // Priced from the amount leaves alone, so a request costs what a quote for those
        // leaves at the same fee rate says.
        let rate = self.fee_rate().await?;
        let fees = coop_exit_fees(num_amount_leaves, rate);
        let total_fee = fees
            .l1_broadcast_fee_sats
            .saturating_add(fees.user_fee_sats);
        let connector_funding = connector_funding_sats(num_connector_leaves, rate);
        let per_leaf_connector = per_leaf_connector_sats(rate);

        // A partial exit's fee is paid by its fee leaves, so its payout is the
        // amount leaves' full value.
        let payout = if withdraw_all {
            leaves_sum.checked_sub(total_fee).ok_or_else(|| {
                CoopExitError::InvalidInput(format!(
                    "leaves ({leaves_sum} sats) do not cover the coop-exit fee ({total_fee} sats)"
                ))
            })?
        } else {
            leaves_sum
        };
        let dust = withdrawal.script_pubkey().minimal_non_dust().to_sat();
        if payout < dust {
            return Err(CoopExitError::InvalidInput(format!(
                "the payout ({payout} sats) is below the dust limit ({dust} sats)"
            )));
        }

        let need = |inputs: usize| {
            payout
                .saturating_add(connector_funding)
                .saturating_add(fees::fee_sats(
                    rate,
                    coop_exit_tx_weight_wu(inputs as u64, 3),
                ))
        };
        let (selected, held) = self
            .wallet
            .select_coins(&need)
            .await
            .map_err(|e| CoopExitError::Wallet(e.to_string()))?;
        if coop_exit_tx_weight_wu(selected.len() as u64, 3) > MAX_STANDARD_TX_WEIGHT_WU {
            return Err(CoopExitError::Wallet(
                "the exit needs more coins than one standard transaction spends".to_string(),
            ));
        }
        let inputs_sum: u64 = selected.iter().map(|u| u.value).sum();

        let connector_funding_addr = self
            .wallet
            .next_address()
            .await
            .map_err(|e| CoopExitError::Wallet(e.to_string()))?;
        let change_addr = self
            .wallet
            .next_address()
            .await
            .map_err(|e| CoopExitError::Wallet(e.to_string()))?;

        let coop_inputs: Vec<TxIn> = selected
            .iter()
            .map(|utxo| TxIn {
                previous_output: utxo.outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            })
            .collect();

        // Connector funding stays at output 1: the connector tx and
        // `miner_fees_sats` refer to it by index.
        let mut coop_outputs = vec![
            TxOut {
                value: Amount::from_sat(payout),
                script_pubkey: withdrawal.script_pubkey(),
            },
            TxOut {
                value: Amount::from_sat(connector_funding),
                script_pubkey: connector_funding_addr.script_pubkey(),
            },
        ];
        let change = inputs_sum.saturating_sub(need(selected.len()));
        if change >= fees::P2TR_DUST_SATS {
            coop_outputs.push(TxOut {
                value: Amount::from_sat(change),
                script_pubkey: change_addr.script_pubkey(),
            });
        }

        let coop_exit_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: coop_inputs,
            output: coop_outputs,
        };
        let coop_exit_txid = coop_exit_tx.compute_txid();

        // The leaf connectors pay the SSP: broadcasting a leaf's refund takes a
        // signature for its connector input.
        let connector_out_addr = self
            .wallet
            .next_address()
            .await
            .map_err(|e| CoopExitError::Wallet(e.to_string()))?;
        let connector_spk = connector_out_addr.script_pubkey();
        let mut connector_outputs: Vec<TxOut> = (0..num_connector_leaves)
            .map(|_| TxOut {
                value: Amount::from_sat(per_leaf_connector),
                script_pubkey: connector_spk.clone(),
            })
            .collect();
        // The operators require one output more than there are leaves and let no
        // refund spend it. It pays the SSP rather than being a zero-value anchor:
        // nodes refuse a transaction that pays a fee and has a dust output.
        connector_outputs.push(TxOut {
            value: Amount::from_sat(fees::P2TR_DUST_SATS),
            script_pubkey: connector_spk,
        });
        let connector_tx = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: coop_exit_txid,
                    vout: 1,
                },
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: connector_outputs,
        };

        // An exit costing the SSP more in miner fees than it charges is refused
        // before the user commits leaves to it.
        let miner_fees = miner_fees_sats(inputs_sum, &coop_exit_tx, &connector_tx);
        if miner_fees > total_fee {
            return Err(CoopExitError::Wallet(format!(
                "the exit's miner fees ({miner_fees} sats) exceed its fee ({total_fee} sats)"
            )));
        }

        let prevouts = selected
            .iter()
            .map(|utxo| CoopExitPrevout {
                txid: utxo.outpoint.txid.to_string(),
                vout: utxo.outpoint.vout,
                value: utxo.value,
                address: utxo.address.to_string(),
            })
            .collect();

        Ok((
            BuiltCoopExitTxs {
                raw_coop_exit_tx: bitcoin::consensus::serialize(&coop_exit_tx),
                raw_connector_tx: bitcoin::consensus::serialize(&connector_tx),
                coop_exit_txid: coop_exit_txid.to_string(),
                amount_sats: payout,
                fee_sats: total_fee,
                prevouts,
            },
            held,
        ))
    }

    /// Returns the signed tx and its txid. Signing a record again yields the same
    /// bytes.
    pub async fn sign_coop_exit_tx(
        &self,
        record: &CoopExitRecord,
    ) -> Result<(Vec<u8>, String), CoopExitError> {
        use std::collections::HashMap;

        use bitcoin::hashes::Hash;
        use bitcoin::key::TapTweak;
        use bitcoin::secp256k1::{Keypair, Message, Secp256k1};
        use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
        use bitcoin::{
            Address, Amount, OutPoint, Transaction, TxOut, Witness, address::NetworkUnchecked,
        };

        let mut tx: Transaction = bitcoin::consensus::deserialize(&record.raw_coop_exit_tx)
            .map_err(|e| {
                CoopExitError::InvalidInput(format!("invalid unsigned coop-exit tx: {e}"))
            })?;
        let network = self.wallet.network();

        let mut prevout_by_outpoint: HashMap<OutPoint, (TxOut, Address)> = HashMap::new();
        for p in &record.prevouts {
            let txid = p
                .txid
                .parse()
                .map_err(|e| CoopExitError::InvalidInput(format!("invalid prevout txid: {e}")))?;
            let address = p
                .address
                .parse::<Address<NetworkUnchecked>>()
                .map_err(|e| CoopExitError::InvalidInput(format!("invalid prevout address: {e}")))?
                .require_network(network)
                .map_err(|e| {
                    CoopExitError::InvalidInput(format!("prevout address wrong network: {e}"))
                })?;
            let txout = TxOut {
                value: Amount::from_sat(p.value),
                script_pubkey: address.script_pubkey(),
            };
            prevout_by_outpoint.insert(OutPoint { txid, vout: p.vout }, (txout, address));
        }

        let mut prevouts: Vec<TxOut> = Vec::with_capacity(tx.input.len());
        let mut keys = Vec::with_capacity(tx.input.len());
        for input in &tx.input {
            let (txout, address) =
                prevout_by_outpoint
                    .get(&input.previous_output)
                    .ok_or_else(|| {
                        CoopExitError::InvalidInput(format!(
                            "missing prevout for input {}",
                            input.previous_output
                        ))
                    })?;
            prevouts.push(txout.clone());
            let secret_key = self
                .wallet
                .derive_keypair_for_address(address)
                .await
                .map_err(|e| CoopExitError::Wallet(e.to_string()))?;
            keys.push(secret_key);
        }

        let secp = Secp256k1::new();
        let mut signatures = Vec::with_capacity(tx.input.len());
        {
            let mut cache = SighashCache::new(&tx);
            for (i, secret_key) in keys.iter().enumerate() {
                let keypair = Keypair::from_secret_key(&secp, secret_key);
                let tweaked = keypair.tap_tweak(&secp, None);
                let sighash = cache
                    .taproot_key_spend_signature_hash(
                        i,
                        &Prevouts::All(&prevouts),
                        TapSighashType::Default,
                    )
                    .map_err(|e| CoopExitError::InvalidInput(format!("sighash error: {e}")))?;
                let msg = Message::from_digest(*sighash.as_byte_array());
                signatures.push(secp.sign_schnorr_no_aux_rand(&msg, &tweaked.to_keypair()));
            }
        }
        for (input, sig) in tx.input.iter_mut().zip(&signatures) {
            input.witness = Witness::from_slice(&[sig.as_ref()]);
        }

        let txid = tx.compute_txid().to_string();
        Ok((bitcoin::consensus::serialize(&tx), txid))
    }

    /// The value of each of `leaf_external_ids`, refusing any that is not an available
    /// leaf of `owner`.
    async fn owned_leaf_values(
        &self,
        leaf_external_ids: &[Uuid],
        owner: &PublicKey,
    ) -> Result<HashMap<Uuid, u64>, CoopExitError> {
        use spark::operator::rpc::spark::{
            QueryNodesRequest, TreeNodeIds, query_nodes_request::Source,
        };
        use spark::tree::{TreeNode, TreeNodeStatus};

        let node_ids: Vec<String> = leaf_external_ids.iter().map(ToString::to_string).collect();
        let net: spark::operator::rpc::spark::Network = self.network.into();
        let resp = crate::operator_rpc::query_nodes_internal(
            &self.operator_pool.get_coordinator().client,
            QueryNodesRequest {
                source: Some(Source::NodeIds(TreeNodeIds {
                    node_ids: node_ids.clone(),
                })),
                include_parents: false,
                limit: i64::try_from(node_ids.len()).unwrap_or(i64::MAX),
                offset: 0,
                network: net as i32,
                statuses: vec![],
            },
        )
        .await
        .map_err(|e| CoopExitError::InvalidInput(format!("query_nodes failed: {e}")))?;

        let mut values = HashMap::with_capacity(leaf_external_ids.len());
        for id in leaf_external_ids {
            let node = resp
                .nodes
                .get(&id.to_string())
                .cloned()
                .ok_or_else(|| CoopExitError::InvalidInput(format!("leaf {id} not found")))?;
            let node = TreeNode::try_from(node).map_err(|e| {
                CoopExitError::InvalidInput(format!("leaf {id} is unreadable: {e}"))
            })?;
            if node.owner_identity_public_key != Some(*owner)
                || node.status != TreeNodeStatus::Available
            {
                return Err(CoopExitError::InvalidInput(format!(
                    "leaf {id} is not an available leaf of the caller"
                )));
            }
            values.insert(*id, node.value);
        }
        Ok(values)
    }

    pub async fn committed_transfer_is_valid(
        &self,
        record: &CoopExitRecord,
    ) -> Result<bool, CoopExitError> {
        match self.verify_committed(record).await {
            Ok(()) => Ok(true),
            Err(CoopExitError::InvalidInput(_) | CoopExitError::TransferNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn verify_committed(&self, record: &CoopExitRecord) -> Result<(), CoopExitError> {
        let incoming = IncomingTransfer::query(
            &self.operator_pool,
            self.signer.as_ref(),
            self.network,
            &record.user_transfer_id,
        )
        .await?
        .ok_or_else(|| CoopExitError::TransferNotFound(record.user_transfer_id.to_string()))?;
        let ssp_pubkey = spark::signer::derive_identity_public_key(self.signer.as_ref()).await?;
        verify_committed_transfer(record, &incoming.transfer, &ssp_pubkey)?;
        if !incoming
            .is_claimable(&self.transfer_service, self.signer.as_ref())
            .await
        {
            return Err(CoopExitError::InvalidInput(
                "the SSP could not claim the transfer's leaves".to_string(),
            ));
        }
        Ok(())
    }
}

/// The user names the exit txid to the operators, so only refunds spending
/// `record`'s connector transaction, whose txid commits to `record`'s exit
/// transaction, bind a transfer to this request.
pub fn verify_committed_transfer(
    record: &CoopExitRecord,
    transfer: &Transfer,
    ssp_pubkey: &PublicKey,
) -> Result<(), CoopExitError> {
    let refuse = |reason: &str| Err(CoopExitError::InvalidInput(reason.to_string()));
    if transfer.transfer_type != TransferType::CooperativeExit {
        return refuse("transfer is not a cooperative exit");
    }
    if transfer.status != TransferStatus::SenderKeyTweakPending {
        return refuse("transfer is not waiting on the exit transaction");
    }
    if transfer.sender_identity_public_key != record.user_identity_public_key {
        return refuse("transfer is not from the requesting user");
    }
    if transfer.receiver_identity_public_key != *ssp_pubkey {
        return refuse("transfer is not addressed to the SSP");
    }

    let unreadable = |e: bitcoin::consensus::encode::Error| {
        CoopExitError::InvalidInput(format!("stored transaction is unreadable: {e}"))
    };
    let connector: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&record.raw_connector_tx).map_err(unreadable)?;
    let exit_tx: bitcoin::Transaction =
        bitcoin::consensus::deserialize(&record.raw_coop_exit_tx).map_err(unreadable)?;

    // The connector's last output is not a leaf's.
    let connector_txid = connector.compute_txid();
    let leaf_outputs = connector.output.len().saturating_sub(1);
    let mut bound = HashSet::new();
    for leaf in &transfer.leaves {
        let connector_input = leaf
            .intermediate_refund_tx
            .input
            .get(1)
            .map(|input| input.previous_output);
        match connector_input {
            Some(outpoint)
                if outpoint.txid == connector_txid
                    && (outpoint.vout as usize) < leaf_outputs
                    && bound.insert(outpoint.vout) => {}
            _ => return refuse("a leaf's refund is not bound to this request's connector"),
        }
    }

    // The SSP requires the payout plus the miner fees it pays, not the fee the
    // request charged.
    let spent: u64 = record.prevouts.iter().map(|p| p.value).sum();
    let required = record
        .amount_sats
        .saturating_add(miner_fees_sats(spent, &exit_tx, &connector));
    if transfer.total_value < required {
        return Err(CoopExitError::InvalidInput(format!(
            "committed transfer value ({} sats) does not cover the payout plus miner fees ({required} sats)",
            transfer.total_value
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
pub trait CoopExitExecutor: Send + Sync {
    /// Signing a record again yields the same bytes.
    async fn sign(&self, record: &CoopExitRecord) -> Result<(Vec<u8>, String), BoxError>;
    async fn broadcast(&self, signed_tx: &[u8]) -> Result<(), BoxError>;
    /// Returns `false` when the transfer is not claimable.
    async fn claim(&self, transfer_id: &TransferId) -> Result<bool, BoxError>;
    async fn committed_transfer_is_valid(&self, record: &CoopExitRecord) -> Result<bool, BoxError>;
    async fn exit_confirmations(&self, record: &CoopExitRecord) -> Result<u64, BoxError>;
}

pub struct CoopExitWorkerDeps {
    pub store: Arc<dyn CoopExitStore>,
    pub executor: Arc<dyn CoopExitExecutor>,
}

pub async fn run_coop_exit_loop(
    deps: CoopExitWorkerDeps,
    wakeup: Wakeup,
    token: CancellationToken,
) {
    info!("Starting coop-exit loop");
    loop {
        if let Err(e) = process_pending_coop_exits(&deps).await {
            error!("Coop-exit check failed: {e}");
        }
        tokio::select! {
            () = token.cancelled() => {
                info!("Coop-exit loop cancelled");
                return;
            }
            () = wakeup.waited() => {}
            () = tokio::time::sleep(COOP_EXIT_BACKUP_INTERVAL) => {}
        }
    }
}

pub async fn process_pending_coop_exits(deps: &CoopExitWorkerDeps) -> Result<(), BoxError> {
    let cutoff = chrono::Duration::from_std(INCOMPLETE_REQUEST_TTL)
        .ok()
        .and_then(|ttl| chrono::Utc::now().checked_sub_signed(ttl))
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);
    for record in deps.store.incomplete_before(cutoff).await? {
        if let Err(e) = settle_incomplete(deps, &record).await {
            error!(coop_exit_id = %record.id, "failed to settle incomplete coop exit: {e}");
        }
    }
    for record in deps.store.pending().await? {
        if let Err(e) = process_coop_exit(deps, &record).await {
            error!(coop_exit_id = %record.id, "failed to advance coop exit: {e}");
        }
    }
    Ok(())
}

async fn settle_incomplete(
    deps: &CoopExitWorkerDeps,
    record: &CoopExitRecord,
) -> Result<(), BoxError> {
    if deps.executor.committed_transfer_is_valid(record).await? {
        deps.store.set_completed(&record.id).await?;
        info!(coop_exit_id = %record.id, "coop exit: completed on the user's behalf");
    } else {
        deps.store.abandon(&record.id).await?;
        info!(coop_exit_id = %record.id, "coop exit: abandoned, its coins released");
    }
    Ok(())
}

async fn process_coop_exit(
    deps: &CoopExitWorkerDeps,
    record: &CoopExitRecord,
) -> Result<(), BoxError> {
    // A node refuses a confirmed transaction once all its outputs are spent, so a
    // refused broadcast still goes on to the claim.
    let (signed, txid) = deps.executor.sign(record).await?;
    match deps.executor.broadcast(&signed).await {
        Ok(()) if record.broadcast_txid.is_none() => {
            deps.store.set_broadcast_txid(&record.id, &txid).await?;
            info!(coop_exit_id = %record.id, %txid, "coop exit: broadcast");
        }
        Ok(()) => {}
        Err(e) => warn!(coop_exit_id = %record.id, %txid, "coop exit: broadcast refused: {e}"),
    }
    let mut leaves_claimed = record.leaves_claimed;
    if !leaves_claimed && deps.executor.claim(&record.user_transfer_id).await? {
        deps.store.set_leaves_claimed(&record.id).await?;
        info!(coop_exit_id = %record.id, "coop exit: claimed user leaves");
        leaves_claimed = true;
    }
    if leaves_claimed
        && deps.executor.exit_confirmations(record).await? >= EXIT_SETTLED_CONFIRMATIONS
    {
        deps.store.set_settled(&record.id).await?;
    }
    Ok(())
}

/// Claims the user's committed leaf transfer into the SSP pool, mirroring the
/// swap claimer: query the transfer, and once it is claimable, claim it and add
/// its leaves to the pool. Returns whether the leaves were claimed (`false` =
/// not yet claimable, retry later). A transfer the operators already finalized
/// is claimed again, which hands back the leaves the SSP still owns: a claim that
/// finished at the operators but not here is finished on a later pass.
pub async fn claim_user_transfer(
    transfer_service: &TransferService,
    tree_store: &Arc<dyn TreeStore>,
    leaf_signing_keys: &Arc<dyn LeafSigningKeys>,
    transfer_id: &TransferId,
) -> Result<bool, BoxError> {
    let Some(transfer) = transfer_service.query_transfer(transfer_id).await? else {
        return Ok(false);
    };
    if !is_claimable(transfer.status) {
        return Ok(false);
    }
    claim_into_pool(
        transfer_service,
        leaf_signing_keys.as_ref(),
        tree_store.as_ref(),
        &transfer,
    )
    .await?;
    Ok(true)
}

/// Includes `Completed`: claiming again hands back the leaves the SSP still owns,
/// finishing a claim that completed at the operators but not here.
fn is_claimable(status: TransferStatus) -> bool {
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

#[cfg(test)]
mod tests {
    use super::repository::{CoopExitRecord, CoopExitStore, InMemoryCoopExitStore};
    use super::{
        BoxError, CoopExitExecutor, CoopExitWorkerDeps, MAX_COOP_EXIT_LEAVES, TransferId,
        check_leaf_count, connector_tx_weight_wu, process_pending_coop_exits,
    };
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn the_most_leaves_an_exit_carries_fit_a_relayable_connector_tx() {
        let one_more = MAX_COOP_EXIT_LEAVES.saturating_add(1);
        assert!(connector_tx_weight_wu(MAX_COOP_EXIT_LEAVES) <= 40_000);
        assert!(connector_tx_weight_wu(one_more) > 40_000);
        assert!(check_leaf_count(MAX_COOP_EXIT_LEAVES).is_ok());
        assert!(check_leaf_count(one_more).is_err());
    }

    fn pubkey(byte: u8) -> PublicKey {
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).expect("secret"))
    }

    #[derive(Default)]
    struct StubExecutor {
        signs: AtomicUsize,
        broadcasts: AtomicUsize,
        claims: AtomicUsize,
        confirmations: AtomicUsize,
        refuse_broadcasts: bool,
    }

    #[async_trait::async_trait]
    impl CoopExitExecutor for StubExecutor {
        async fn sign(&self, record: &CoopExitRecord) -> Result<(Vec<u8>, String), BoxError> {
            self.signs.fetch_add(1, Ordering::SeqCst);
            Ok((
                record.raw_coop_exit_tx.clone(),
                format!("txid-{}", record.raw_coop_exit_tx.len()),
            ))
        }
        async fn broadcast(&self, _signed_tx: &[u8]) -> Result<(), BoxError> {
            self.broadcasts.fetch_add(1, Ordering::SeqCst);
            if self.refuse_broadcasts {
                return Err("bad-txns-inputs-missingorspent".into());
            }
            Ok(())
        }
        async fn claim(&self, _transfer_id: &TransferId) -> Result<bool, BoxError> {
            let claims = self.claims.fetch_add(1, Ordering::SeqCst);
            Ok(claims > 0)
        }
        async fn committed_transfer_is_valid(
            &self,
            _record: &CoopExitRecord,
        ) -> Result<bool, BoxError> {
            Ok(false)
        }
        async fn exit_confirmations(&self, _record: &CoopExitRecord) -> Result<u64, BoxError> {
            Ok(self.confirmations.load(Ordering::SeqCst) as u64)
        }
    }

    fn record(id: &str) -> CoopExitRecord {
        CoopExitRecord {
            id: id.to_string(),
            user_identity_public_key: pubkey(1),
            user_transfer_id: TransferId::generate(),
            withdrawal_address: "bcrt1qexample".to_string(),
            amount_sats: 10_000,
            fee_sats: 500,
            raw_coop_exit_tx: vec![1, 2, 3, 4],
            raw_connector_tx: vec![5, 6],
            coop_exit_txid: "txid-4".to_string(),
            prevouts: Vec::new(),
            completed: false,
            broadcast_txid: None,
            leaves_claimed: false,
            settled: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn worker_rebroadcasts_until_the_exit_is_buried() {
        let store = Arc::new(InMemoryCoopExitStore::default());
        let executor = Arc::new(StubExecutor::default());
        let deps = CoopExitWorkerDeps {
            store: Arc::clone(&store) as Arc<dyn CoopExitStore>,
            executor: Arc::clone(&executor) as Arc<dyn CoopExitExecutor>,
        };

        store.insert(&record("c1"), &[]).await.unwrap();
        process_pending_coop_exits(&deps).await.unwrap();
        assert_eq!(executor.broadcasts.load(Ordering::SeqCst), 0);

        store.set_completed("c1").await.unwrap();
        process_pending_coop_exits(&deps).await.unwrap();
        assert_eq!(executor.broadcasts.load(Ordering::SeqCst), 1);
        let after_broadcast = store.get("c1").await.unwrap().unwrap();
        assert!(after_broadcast.broadcast_txid.is_some());
        assert!(!after_broadcast.leaves_claimed);

        process_pending_coop_exits(&deps).await.unwrap();
        assert_eq!(executor.broadcasts.load(Ordering::SeqCst), 2);
        assert!(store.get("c1").await.unwrap().unwrap().leaves_claimed);

        process_pending_coop_exits(&deps).await.unwrap();
        assert_eq!(executor.broadcasts.load(Ordering::SeqCst), 3);
        assert_eq!(executor.claims.load(Ordering::SeqCst), 2);

        executor.confirmations.store(6, Ordering::SeqCst);
        process_pending_coop_exits(&deps).await.unwrap();
        assert!(store.get("c1").await.unwrap().unwrap().settled);
        process_pending_coop_exits(&deps).await.unwrap();
        assert_eq!(executor.broadcasts.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_leaf_backs_one_open_request_at_a_time() {
        let store = InMemoryCoopExitStore::default();
        let leaves = vec!["leaf".to_string()];
        store.insert(&record("c1"), &leaves).await.unwrap();
        assert!(store.any_leaf_in_open_request(&leaves).await.unwrap());
        assert!(store.insert(&record("c2"), &leaves).await.is_err());

        store.set_completed("c1").await.unwrap();
        assert!(!store.any_leaf_in_open_request(&leaves).await.unwrap());
    }

    #[tokio::test]
    async fn a_refused_broadcast_does_not_hold_up_the_claim() {
        let store = Arc::new(InMemoryCoopExitStore::default());
        let executor = Arc::new(StubExecutor {
            refuse_broadcasts: true,
            ..StubExecutor::default()
        });
        let deps = CoopExitWorkerDeps {
            store: Arc::clone(&store) as Arc<dyn CoopExitStore>,
            executor: Arc::clone(&executor) as Arc<dyn CoopExitExecutor>,
        };
        store.insert(&record("c1"), &[]).await.unwrap();
        store.set_completed("c1").await.unwrap();

        process_pending_coop_exits(&deps).await.unwrap();
        process_pending_coop_exits(&deps).await.unwrap();
        assert!(store.get("c1").await.unwrap().unwrap().leaves_claimed);
    }

    #[tokio::test]
    async fn an_incomplete_request_without_a_transfer_is_abandoned_after_its_ttl() {
        let store = Arc::new(InMemoryCoopExitStore::default());
        let executor = Arc::new(StubExecutor::default());
        let deps = CoopExitWorkerDeps {
            store: Arc::clone(&store) as Arc<dyn CoopExitStore>,
            executor: Arc::clone(&executor) as Arc<dyn CoopExitExecutor>,
        };
        let mut old = record("old");
        old.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
        store.insert(&old, &[]).await.unwrap();
        store.insert(&record("new"), &[]).await.unwrap();

        process_pending_coop_exits(&deps).await.unwrap();
        assert!(store.get("old").await.unwrap().is_none());
        assert!(store.get("new").await.unwrap().is_some());
    }
}
