//! Checking the leaves of a transfer to the SSP before the SSP provides a
//! service for it.
//!
//! The SSP reads what it has stored first. The leaf store has the exit chains of
//! the SSP's own leaves, and of each leaf the SSP has verified before. The spend
//! check records in the chain record whether the inputs on those chains are
//! spent. The SSP queries the coordinator only for a leaf whose exit chain the
//! store does not have. It queries bitcoind only when the chain record does not
//! have what the check needs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Message, Secp256k1, XOnlyPublicKey, schnorr};
use bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
use bitcoin::{OutPoint, Transaction, TxOut, Txid};
use spark::operator::OperatorPool;
use spark::services::{Transfer, csv_timelock};
use spark::tree::{LeafPedigree, TreeNode};
use tracing::warn;

use crate::chain::spend_check::MAX_OUTPUTS_PER_QUERY;
use crate::chain::{
    CallQueue, ChainError, ChainTip, Synced, TransactionConfirmations, UnspentOutput,
};
use crate::leaves::{AncestorsError, with_ancestors};
use crate::operator_rpc::NodeQuery;
use crate::pool::unilateral_exit::initial_timelock;
use crate::postgresql::{
    ChainRepository, ChainTransaction, ExitChainStep, PostgresLeafStore, StoredChains, exit_chain,
    exiting_steps, refund_step,
};

#[derive(Debug, thiserror::Error)]
pub enum IncomingLeafError {
    #[error("{0}")]
    Refused(String),

    /// The SSP could not check the leaves. Its log has the reason.
    #[error("the SSP could not serve the request")]
    Unavailable,
}

fn refused(leaf: &TreeNode, why: &str) -> IncomingLeafError {
    IncomingLeafError::Refused(format!("leaf {} {why}", leaf.id))
}

fn unavailable(why: impl std::fmt::Display) -> IncomingLeafError {
    warn!("could not check a transfer's leaves: {why}");
    IncomingLeafError::Unavailable
}

#[async_trait::async_trait]
pub trait Ancestors: Send + Sync {
    /// Returns each of `leaves` with every node above it up to its tree's root.
    async fn of(&self, leaves: &[TreeNode]) -> Result<Vec<LeafPedigree>, AncestorsError>;
}

pub struct CoordinatorAncestors {
    pub operator_pool: Arc<OperatorPool>,
    pub network: spark::Network,
}

#[async_trait::async_trait]
impl Ancestors for CoordinatorAncestors {
    async fn of(&self, leaves: &[TreeNode]) -> Result<Vec<LeafPedigree>, AncestorsError> {
        with_ancestors(
            &self.operator_pool,
            self.network,
            leaves,
            NodeQuery::Internal,
        )
        .await
    }
}

/// The SSP accepts a leaf that it could exit unilaterally, provided the
/// operators sign no other transaction that has the tree's funding output, or
/// an output on the leaf's exit chain, as its input.
pub struct IncomingLeafCheck {
    pub leaf_store: Arc<PostgresLeafStore>,
    pub chain_repository: Arc<ChainRepository>,
    pub chain_tip: ChainTip,
    pub ancestors: Arc<dyn Ancestors>,
    pub call_queue: Arc<CallQueue>,
    pub synced: Synced,
    /// The number of confirmations the SSP requires of a tree's funding output.
    pub confirmations: u64,
    /// How long the SSP waits for the bitcoind calls of one check.
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Spent {
    by: Txid,
    /// `None` while the spender is in the mempool.
    height: Option<u64>,
}

struct ExitChain<'a> {
    leaf: &'a TreeNode,
    /// The leaf's refund first, then each node transaction up to the root's.
    steps: Vec<ExitChainStep>,
    /// The ids of the node transactions, from the leaf's up to the root's.
    transactions: Vec<Txid>,
}

impl ExitChain<'_> {
    fn funding(&self) -> Result<OutPoint, IncomingLeafError> {
        self.steps
            .last()
            .filter(|_| self.steps.len() > 1)
            .map(|root| root.spends)
            .ok_or_else(|| refused(self.leaf, "is in a tree whose root has no input"))
    }

    fn root(&self) -> Option<Txid> {
        self.transactions.last().copied()
    }
}

/// What bitcoind reported for a tree's funding output and, when that output is
/// spent or missing, for the outputs on the exit chains below it.
struct FundingOutputResult {
    unspent: Option<UnspentOutput>,
    spends: HashMap<OutPoint, Spent>,
}

impl IncomingLeafCheck {
    pub async fn check(&self, transfer: &Transfer) -> Result<(), IncomingLeafError> {
        let leaves: Vec<&TreeNode> = transfer.leaves.iter().map(|l| &l.leaf).collect();
        self.check_leaves(&leaves).await
    }

    async fn check_leaves(&self, leaves: &[&TreeNode]) -> Result<(), IncomingLeafError> {
        if !self.synced.is_synced() {
            return Err(unavailable("the chain record is not at bitcoind's tip"));
        }
        let node_txids: Vec<Txid> = leaves
            .iter()
            .map(|leaf| leaf.node_tx.compute_txid())
            .collect();
        let stored = self
            .leaf_store
            .stored_chains(&node_txids)
            .await
            .map_err(unavailable)?;
        let tip_height = self.tip_height();

        let mut unresolved: Vec<ExitChain<'_>> = Vec::new();
        let mut not_stored: Vec<TreeNode> = Vec::new();
        for (leaf, node_txid) in leaves.iter().zip(&node_txids) {
            let Some(transactions) = stored.chains.get(node_txid) else {
                not_stored.push((*leaf).clone());
                continue;
            };
            check_refund(leaf)?;
            let chain = ExitChain::stored(leaf, transactions);
            if !resolved_by_record(&chain, &stored, tip_height, self.confirmations)? {
                unresolved.push(chain);
            }
        }
        if unresolved.is_empty() && not_stored.is_empty() {
            return Ok(());
        }
        self.check_unresolved(unresolved, &not_stored).await
    }

    fn tip_height(&self) -> u64 {
        self.chain_tip.record().map_or(0, |tip| tip.height)
    }

    async fn check_unresolved(
        &self,
        stored: Vec<ExitChain<'_>>,
        not_stored: &[TreeNode],
    ) -> Result<(), IncomingLeafError> {
        let pedigrees = self.verified_pedigrees(not_stored).await?;
        let reported: Vec<ExitChain<'_>> = pedigrees.iter().map(ExitChain::of).collect();
        let roots: Vec<Txid> = reported.iter().filter_map(ExitChain::root).collect();
        let stored_roots = self
            .leaf_store
            .stored_chains(&roots)
            .await
            .map_err(unavailable)?;
        let tip_height = self.tip_height();
        let (resolved, to_query) = funding_outputs(&stored, &reported, &stored_roots, tip_height)?;

        let chains = || stored.iter().chain(&reported);
        let querying = self.query_bitcoind(&to_query, chains());
        let results = match tokio::time::timeout(self.timeout, querying).await {
            Ok(Ok(results)) => results,
            Ok(Err(e)) => return Err(unavailable(e)),
            Err(_) => return Err(unavailable("bitcoind returned no result in time")),
        };

        // The store has only transactions with a valid signature, so the SSP
        // verifies the root of a reported exit chain only when the store does
        // not have that root.
        let roots = stored.iter().map(|_| None).chain(pedigrees.iter().map(|p| {
            Some(p.ancestors.last().unwrap_or(&p.leaf)).filter(|root| {
                !stored_roots
                    .chains
                    .contains_key(&root.node_tx.compute_txid())
            })
        }));
        let unspent = HashMap::new();
        for (chain, unverified_root) in chains().zip(roots) {
            let funding = chain.funding()?;
            if let Some(confirmations) = resolved.get(&funding) {
                let confirmations = Some(*confirmations);
                validate(
                    chain,
                    &unspent,
                    confirmations,
                    tip_height,
                    self.confirmations,
                )?;
                continue;
            }
            let Some(result) = results.get(&funding) else {
                return Err(unavailable("bitcoind was not queried for a funding output"));
            };
            validate_with_bitcoind(
                chain,
                unverified_root,
                result,
                tip_height,
                self.confirmations,
            )?;
        }
        self.keep(&pedigrees, &results).await
    }

    async fn verified_pedigrees(
        &self,
        leaves: &[TreeNode],
    ) -> Result<Vec<LeafPedigree>, IncomingLeafError> {
        if leaves.is_empty() {
            return Ok(Vec::new());
        }
        let pedigrees = match self.ancestors.of(leaves).await {
            Ok(pedigrees) => pedigrees,
            Err(e @ AncestorsError::MissingAncestors(_)) => {
                return Err(IncomingLeafError::Refused(e.to_string()));
            }
            Err(e @ AncestorsError::Query(_)) => return Err(unavailable(e)),
        };
        if pedigrees.len() != leaves.len() {
            return Err(IncomingLeafError::Refused(
                "the operators do not report every leaf of the transfer".to_string(),
            ));
        }
        for pedigree in &pedigrees {
            verify_pedigree(pedigree)?;
        }
        Ok(pedigrees)
    }

    /// Stores the exit chains the SSP verified, and the block of each funding
    /// transaction that bitcoind reported as confirmed, so that the SSP can
    /// check a leaf below them later without a call.
    async fn keep(
        &self,
        pedigrees: &[LeafPedigree],
        results: &HashMap<OutPoint, FundingOutputResult>,
    ) -> Result<(), IncomingLeafError> {
        if !pedigrees.is_empty() {
            self.leaf_store
                .store_nodes(pedigrees)
                .await
                .map_err(unavailable)?;
        }
        let confirmed: Vec<TransactionConfirmations> = results
            .iter()
            .filter_map(|(funding, result)| {
                let output = result.unspent.as_ref()?;
                Some(TransactionConfirmations {
                    txid: funding.txid,
                    best_block: output.best_block,
                    confirmations: output.confirmations,
                })
            })
            .collect();
        if let Err(e) = self.chain_repository.record_tx_blocks(&confirmed).await {
            warn!("could not record the blocks of funding transactions: {e}");
        }
        Ok(())
    }

    async fn query_bitcoind<'a, 'b: 'a>(
        &self,
        funding: &[OutPoint],
        chains: impl Iterator<Item = &'a ExitChain<'b>> + Clone,
    ) -> Result<HashMap<OutPoint, FundingOutputResult>, ChainError> {
        let mut results = HashMap::with_capacity(funding.len());
        for output in funding {
            let unspent = self.call_queue.unspent_output(output).await?;
            let mut spends = HashMap::new();
            if unspent.is_none() {
                let below: HashSet<OutPoint> = chains
                    .clone()
                    .filter(|chain| chain.funding().is_ok_and(|funding| funding == *output))
                    .flat_map(|chain| chain.steps.iter().map(|step| step.spends))
                    .collect();
                let below: Vec<OutPoint> = below.into_iter().collect();
                for chunk in below.chunks(MAX_OUTPUTS_PER_QUERY as usize) {
                    for spend in self.call_queue.get_spends(chunk).await? {
                        spends.insert(
                            spend.outpoint,
                            Spent {
                                by: spend.spender,
                                height: spend.block.map(|block| block.height),
                            },
                        );
                    }
                }
            }
            results.insert(*output, FundingOutputResult { unspent, spends });
        }
        Ok(results)
    }
}

impl<'a> ExitChain<'a> {
    fn stored(leaf: &'a TreeNode, transactions: &[ChainTransaction]) -> Self {
        let mut steps = vec![refund_step(leaf)];
        steps.extend(transactions.iter().map(|transaction| transaction.step));
        Self {
            leaf,
            steps,
            transactions: transactions.iter().map(|t| t.txid).collect(),
        }
    }

    fn of(pedigree: &'a LeafPedigree) -> Self {
        Self {
            leaf: &pedigree.leaf,
            steps: exit_chain(pedigree),
            transactions: std::iter::once(&pedigree.leaf)
                .chain(&pedigree.ancestors)
                .map(|node| node.node_tx.compute_txid())
                .collect(),
        }
    }
}

/// Splits the funding outputs of `stored` and `reported` into those the SSP
/// resolves from the chain record, each with its confirmations, and those it
/// has to query bitcoind for. The SSP could not resolve any chain of `stored`
/// from the chain record.
///
/// A reported exit chain can end in a root the store has, for example when
/// someone renewed the leaf. When the chain record has that tree's funding
/// output as confirmed and unspent, no transaction below that output can be on
/// chain, so the SSP needs no call to bitcoind for that tree.
fn funding_outputs(
    stored: &[ExitChain<'_>],
    reported: &[ExitChain<'_>],
    stored_roots: &StoredChains,
    tip_height: u64,
) -> Result<(HashMap<OutPoint, u64>, Vec<OutPoint>), IncomingLeafError> {
    let mut resolved: HashMap<OutPoint, u64> = HashMap::new();
    let mut to_query: Vec<OutPoint> = Vec::new();
    for chain in stored {
        let funding = chain.funding()?;
        if !to_query.contains(&funding) {
            to_query.push(funding);
        }
    }
    for chain in reported {
        let funding = chain.funding()?;
        if resolved.contains_key(&funding) || to_query.contains(&funding) {
            continue;
        }
        let root_stored = chain
            .root()
            .is_some_and(|root| stored_roots.chains.contains_key(&root));
        match unspent_funding(funding, stored_roots, tip_height).filter(|_| root_stored) {
            Some(confirmations) => {
                resolved.insert(funding, confirmations);
            }
            None => to_query.push(funding),
        }
    }
    Ok((resolved, to_query))
}

/// `unverified_root` is the root when the SSP has no valid signature for it
/// yet. A root that is in a block needs no signature check, because a block
/// contains only valid transactions.
fn validate_with_bitcoind(
    chain: &ExitChain<'_>,
    unverified_root: Option<&TreeNode>,
    result: &FundingOutputResult,
    tip_height: u64,
    confirmations: u64,
) -> Result<(), IncomingLeafError> {
    if let (Some(root), Some(output)) = (unverified_root, &result.unspent)
        && !is_signed_for(&root.node_tx, &output.tx_out)
    {
        return Err(refused(
            chain.leaf,
            "is in a tree whose root has no valid signature",
        ));
    }
    let funding_confirmations = result.unspent.as_ref().map(|output| output.confirmations);
    validate(
        chain,
        &result.spends,
        funding_confirmations,
        tip_height,
        confirmations,
    )
}

/// Returns the confirmations of `funding` when the chain record has it as
/// confirmed and unspent. The chain record watches the funding output of each
/// tree whose root the store has, so the spend check records whether such an
/// output is spent. The record has the funding transaction's block for a tree
/// the SSP made itself, or one the SSP queried bitcoind for before.
fn unspent_funding(funding: OutPoint, stored: &StoredChains, tip_height: u64) -> Option<u64> {
    if stored.states.unchecked.contains(&funding) || stored.states.spent.contains_key(&funding) {
        return None;
    }
    let height = stored.funding_heights.get(&funding.txid)?;
    Some(tip_height.saturating_add(1).saturating_sub(*height))
}

/// Validates a leaf whose exit chain the store has, by the chain record alone.
/// The SSP has a valid signature for each node transaction on that chain.
/// Returns `false` when the spend check has yet to query bitcoind for an output
/// on the chain, or when the record has no block for the tree's funding
/// transaction.
fn resolved_by_record(
    chain: &ExitChain<'_>,
    stored: &StoredChains,
    tip_height: u64,
    confirmations: u64,
) -> Result<bool, IncomingLeafError> {
    if chain
        .steps
        .iter()
        .any(|step| stored.states.unchecked.contains(&step.spends))
    {
        return Ok(false);
    }
    let spends: HashMap<OutPoint, Spent> = chain
        .steps
        .iter()
        .filter_map(|step| stored.states.spent.get(&step.spends))
        .map(|spender| {
            (
                spender.outpoint,
                Spent {
                    by: spender.txid,
                    height: Some(spender.block_height),
                },
            )
        })
        .collect();
    let funding = chain.funding()?;
    let funding_confirmations = if spends.contains_key(&funding) {
        None
    } else {
        let Some(confirmations) = unspent_funding(funding, stored, tip_height) else {
            return Ok(false);
        };
        Some(confirmations)
    };
    validate(
        chain,
        &spends,
        funding_confirmations,
        tip_height,
        confirmations,
    )?;
    Ok(true)
}

/// The SSP signs its own refund at the timelock of the refund in the transfer.
/// That timelock has to be in blocks and at most the initial timelock: with a
/// higher one, the SSP would wait longer than the initial timelock before it
/// could exit the leaf unilaterally. A refund that needs renewal is no reason
/// to refuse the leaf: the SSP renews the leaf after it claims the transfer.
fn check_refund(leaf: &TreeNode) -> Result<(), IncomingLeafError> {
    let refund = leaf
        .refund_tx
        .as_ref()
        .ok_or_else(|| refused(leaf, "has no refund"))?;
    if !has_valid_timelock(refund) {
        return Err(refused(
            leaf,
            "has a transaction whose timelock is not in blocks or is above the initial timelock",
        ));
    }
    Ok(())
}

/// This function does not verify the root's signature. Verifying it needs the
/// funding output, and only the chain has that output.
fn verify_pedigree(pedigree: &LeafPedigree) -> Result<(), IncomingLeafError> {
    let leaf = &pedigree.leaf;
    let nodes: Vec<&TreeNode> = std::iter::once(leaf).chain(&pedigree.ancestors).collect();
    for pair in nodes.windows(2) {
        let (Some(node), Some(parent)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let spent = spent_output(node, parent)
            .ok_or_else(|| refused(leaf, "has a node whose input is no output of its parent"))?;
        if !is_signed_for(&node.node_tx, spent) {
            return Err(refused(
                leaf,
                "has a node transaction without a valid signature",
            ));
        }
    }
    check_refund(leaf)?;
    if !nodes.iter().all(|node| has_valid_timelock(&node.node_tx)) {
        return Err(refused(
            leaf,
            "has a transaction whose timelock is not in blocks or is above the initial timelock",
        ));
    }
    Ok(())
}

fn spent_output<'a>(node: &TreeNode, parent: &'a TreeNode) -> Option<&'a TxOut> {
    node.node_tx
        .input
        .first()
        .filter(|input| input.previous_output.txid == parent.node_tx.compute_txid())
        .and_then(|input| {
            parent
                .node_tx
                .output
                .get(input.previous_output.vout as usize)
        })
}

/// Fails unless the SSP has a valid signature for every node transaction of
/// `pedigree`. The SSP has one for each transaction in `stored`, the ones the
/// store has, and verifies any other against the output above it. The root has
/// to be in `stored`, because only the chain has the output above the root.
pub(crate) fn verify_claimed(
    pedigree: &LeafPedigree,
    stored: &HashSet<Txid>,
) -> Result<(), String> {
    let leaf = &pedigree.leaf;
    let nodes: Vec<&TreeNode> = std::iter::once(leaf).chain(&pedigree.ancestors).collect();
    for pair in nodes.windows(2) {
        let (Some(node), Some(parent)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        if stored.contains(&node.node_tx.compute_txid()) {
            continue;
        }
        if !spent_output(node, parent).is_some_and(|spent| is_signed_for(&node.node_tx, spent)) {
            return Err(format!(
                "leaf {} has a node transaction without a valid signature",
                leaf.id
            ));
        }
    }
    let root = nodes.last().copied().unwrap_or(leaf);
    if !stored.contains(&root.node_tx.compute_txid()) {
        return Err(format!(
            "the SSP has not checked the root of the tree of leaf {}",
            leaf.id
        ));
    }
    Ok(())
}

/// `spends` has the spend of each spent output on the exit chain.
/// `funding_confirmations` is the number of confirmations of the tree's funding
/// output while that output is unspent.
fn validate(
    chain: &ExitChain<'_>,
    spends: &HashMap<OutPoint, Spent>,
    funding_confirmations: Option<u64>,
    tip_height: u64,
    confirmations: u64,
) -> Result<(), IncomingLeafError> {
    let leaf = chain.leaf;
    if exiting_steps(&chain.steps).any(|step| spends.contains_key(&step.spends)) {
        return Err(refused(leaf, "is exiting"));
    }
    for step in &chain.steps {
        if let Some(spent) = spends.get(&step.spends)
            && !chain.transactions.contains(&spent.by)
        {
            return Err(refused(
                leaf,
                "is below an output that a conflicting transaction spends",
            ));
        }
    }

    let funding = chain.funding()?;
    let funding_confirmations = match (spends.get(&funding), funding_confirmations) {
        // The root is the spend of the funding output, so the funding
        // transaction has at least as many confirmations as the root. The SSP
        // uses the root's confirmations for the funding output.
        (Some(spent), _) => spent.height.map_or(0, |height| {
            tip_height.saturating_add(1).saturating_sub(height)
        }),
        (None, Some(funding_confirmations)) => funding_confirmations,
        (None, None) => {
            return Err(refused(
                leaf,
                "is in a tree whose funding output is not on chain",
            ));
        }
    };
    if funding_confirmations < confirmations {
        return Err(refused(
            leaf,
            "is in a tree whose funding output has too few confirmations",
        ));
    }
    Ok(())
}

fn has_valid_timelock(transaction: &Transaction) -> bool {
    let in_blocks = transaction
        .input
        .first()
        .is_some_and(|input| input.sequence.is_height_locked());
    in_blocks && csv_timelock(transaction).unwrap_or(0) <= initial_timelock()
}

fn is_signed_for(transaction: &Transaction, spent: &TxOut) -> bool {
    if !spent.script_pubkey.is_p2tr() || transaction.input.len() != 1 {
        return false;
    }
    let signed = || {
        let key = XOnlyPublicKey::from_slice(spent.script_pubkey.as_bytes().get(2..)?).ok()?;
        let witness = transaction.input.first()?.witness.nth(0)?;
        let signature = schnorr::Signature::from_slice(witness).ok()?;
        let sighash = SighashCache::new(transaction)
            .taproot_key_spend_signature_hash(
                0,
                &Prevouts::All(std::slice::from_ref(spent)),
                TapSighashType::Default,
            )
            .ok()?;
        Secp256k1::verification_only()
            .verify_schnorr(
                &signature,
                &Message::from_digest(*sighash.as_byte_array()),
                &key,
            )
            .ok()
    };
    signed().is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use bitcoin::absolute::LockTime;
    use bitcoin::key::{Keypair, TweakedPublicKey};
    use bitcoin::secp256k1::SecretKey;
    use bitcoin::transaction::Version;
    use bitcoin::{Address, Amount, BlockHash, Network, ScriptBuf, Sequence, TxIn, Witness};
    use deadpool_postgres::Pool;
    use spark::operator::rpc::OperatorRpcError;
    use spark::tree::{SigningKeyshare, TreeNodeStatus, chain_reaches_root};
    use uuid::Uuid;

    use super::*;
    use crate::chain::{
        AddressUtxo, BlockHeader, ChainRepository as _, OutputSpend, RequestChainClient, Txo,
    };
    use crate::postgresql::testing::TestDatabase;

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_slice(&[byte; 32]).unwrap()
    }

    fn output(key: &SecretKey, value: u64) -> TxOut {
        let (xonly, _) = key.public_key(&Secp256k1::new()).x_only_public_key();
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey: ScriptBuf::new_p2tr_tweaked(TweakedPublicKey::dangerous_assume_tweaked(
                xonly,
            )),
        }
    }

    fn signed(
        outpoint: OutPoint,
        spent: &TxOut,
        signer: &SecretKey,
        outputs: Vec<TxOut>,
    ) -> Transaction {
        let mut transaction = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: outpoint,
                sequence: Sequence::ZERO,
                ..Default::default()
            }],
            output: outputs,
        };
        let sighash = SighashCache::new(&transaction)
            .taproot_key_spend_signature_hash(
                0,
                &Prevouts::All(std::slice::from_ref(spent)),
                TapSighashType::Default,
            )
            .unwrap();
        let secp = Secp256k1::new();
        let signature = secp.sign_schnorr_no_aux_rand(
            &Message::from_digest(*sighash.as_byte_array()),
            &Keypair::from_secret_key(&secp, signer),
        );
        transaction.input[0].witness = Witness::from_slice(&[signature.as_ref()]);
        transaction
    }

    /// The refund's signature is not valid: no code under test verifies a
    /// refund's signature.
    fn refund(above: &Transaction, sequence: Sequence) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(above.compute_txid(), 0),
                sequence,
                witness: Witness::from_slice(&[[1u8; 64]]),
                ..Default::default()
            }],
            output: above.output.clone(),
        }
    }

    fn node(id: u128, tree: u8, parent: Option<&TreeNode>, node_tx: Transaction) -> TreeNode {
        let public = key(9).public_key(&Secp256k1::new());
        TreeNode {
            id: Uuid::from_u128(id).to_string().parse().unwrap(),
            tree_id: Uuid::from_u128(u128::from(tree)).to_string(),
            value: node_tx.output.iter().map(|o| o.value.to_sat()).sum(),
            parent_node_id: parent.map(|parent| parent.id.clone()),
            vout: node_tx.input[0].previous_output.vout,
            node_tx,
            refund_tx: None,
            direct_tx: None,
            direct_refund_tx: None,
            direct_from_cpfp_refund_tx: None,
            verifying_public_key: public,
            owner_identity_public_key: Some(public),
            signing_keyshare: SigningKeyshare {
                owner_identifiers: vec![],
                threshold: 2,
                public_key: public,
            },
            status: TreeNodeStatus::Available,
        }
    }

    struct Tree {
        funding: OutPoint,
        funded: TxOut,
        pedigree: LeafPedigree,
    }

    impl Tree {
        /// A tree made with another `funding_byte` has another funding output.
        /// The function signs the root, the node below it and the leaf's node
        /// transaction with the keys in `signers`, in that order. The
        /// signatures are valid only when `signers` is `[1, 2, 3]`.
        fn signed_with(funding_byte: u8, signers: [u8; 3]) -> Self {
            let funding = OutPoint::new(Txid::from_byte_array([funding_byte; 32]), 0);
            let funded = output(&key(1), 3_000);
            let ids = u128::from(funding_byte).saturating_mul(10);
            let root_tx = signed(
                funding,
                &funded,
                &key(signers[0]),
                vec![output(&key(2), 3_000)],
            );
            let root = node(ids, funding_byte, None, root_tx);
            let middle_tx = signed(
                OutPoint::new(root.node_tx.compute_txid(), 0),
                &root.node_tx.output[0],
                &key(signers[1]),
                vec![output(&key(3), 3_000)],
            );
            let middle = node(ids.saturating_add(1), funding_byte, Some(&root), middle_tx);
            let leaf_tx = signed(
                OutPoint::new(middle.node_tx.compute_txid(), 0),
                &middle.node_tx.output[0],
                &key(signers[2]),
                vec![output(&key(4), 3_000)],
            );
            let mut leaf = node(ids.saturating_add(2), funding_byte, Some(&middle), leaf_tx);
            leaf.refund_tx = Some(refund(&leaf.node_tx, Sequence::from_height(2000)));
            Self {
                funding,
                funded,
                pedigree: LeafPedigree {
                    leaf,
                    ancestors: vec![middle, root],
                },
            }
        }

        fn new() -> Self {
            Self::signed_with(7, [1, 2, 3])
        }

        fn root(&self) -> &TreeNode {
            &self.pedigree.ancestors[1]
        }

        fn middle(&self) -> &TreeNode {
            &self.pedigree.ancestors[0]
        }

        fn renewed(&self) -> LeafPedigree {
            let middle = self.middle();
            let leaf_tx = signed(
                OutPoint::new(middle.node_tx.compute_txid(), 0),
                &middle.node_tx.output[0],
                &key(3),
                vec![output(&key(5), 3_000)],
            );
            let mut leaf = self.pedigree.leaf.clone();
            leaf.refund_tx = Some(refund(&leaf_tx, Sequence::from_height(2000)));
            leaf.node_tx = leaf_tx;
            LeafPedigree {
                leaf,
                ancestors: self.pedigree.ancestors.clone(),
            }
        }

        fn validate(
            &self,
            spends: &[(OutPoint, Txid, Option<u64>)],
            funding_confirmations: Option<u64>,
        ) -> Result<(), String> {
            let spends = spends
                .iter()
                .map(|(outpoint, by, height)| {
                    (
                        *outpoint,
                        Spent {
                            by: *by,
                            height: *height,
                        },
                    )
                })
                .collect();
            let chain = ExitChain::of(&self.pedigree);
            validate(&chain, &spends, funding_confirmations, 110, 3).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn a_leaf_with_a_signed_chain_and_a_confirmed_funding_output_passes() {
        let tree = Tree::new();
        assert!(verify_pedigree(&tree.pedigree).is_ok());
        assert!(is_signed_for(&tree.root().node_tx, &tree.funded));
        assert_eq!(tree.validate(&[], Some(3)), Ok(()));
    }

    #[test]
    fn a_funding_output_needs_its_confirmations() {
        let refused = Tree::new().validate(&[], Some(2)).unwrap_err();
        assert!(refused.contains("too few confirmations"), "{refused}");
    }

    #[test]
    fn a_funding_output_that_is_not_on_chain_is_refused() {
        let refused = Tree::new().validate(&[], None).unwrap_err();
        assert!(refused.contains("not on chain"), "{refused}");
    }

    #[test]
    fn every_transaction_below_the_root_needs_the_signature_of_the_key_above_it() {
        for signers in [[1, 8, 3], [1, 2, 8]] {
            let refused = verify_pedigree(&Tree::signed_with(7, signers).pedigree)
                .unwrap_err()
                .to_string();
            assert!(refused.contains("without a valid signature"), "{refused}");
        }
    }

    #[test]
    fn a_node_has_to_have_an_output_of_the_node_above_it_as_its_input() {
        let mut tree = Tree::new();
        let elsewhere = OutPoint::new(Txid::from_byte_array([5; 32]), 0);
        tree.pedigree.leaf.node_tx.input[0].previous_output = elsewhere;

        let refused = verify_pedigree(&tree.pedigree).unwrap_err().to_string();

        assert!(refused.contains("is no output of its parent"), "{refused}");
    }

    #[test]
    fn a_leaf_stays_acceptable_when_only_its_root_has_confirmed() {
        let tree = Tree::new();
        let root = tree.root().node_tx.compute_txid();

        // The root is in the block at height 100, so the funding transaction
        // has at least 11 confirmations at a tip of 110.
        assert_eq!(
            tree.validate(&[(tree.funding, root, Some(100))], None),
            Ok(())
        );

        let refused = tree
            .validate(&[(tree.funding, root, Some(109))], None)
            .unwrap_err();
        assert!(refused.contains("too few confirmations"), "{refused}");
        let refused = tree
            .validate(&[(tree.funding, root, None)], None)
            .unwrap_err();
        assert!(refused.contains("too few confirmations"), "{refused}");
    }

    #[test]
    fn an_exiting_leaf_is_refused() {
        let tree = Tree::new();
        let root = tree.root().node_tx.compute_txid();
        let middle = tree.middle().node_tx.compute_txid();
        let spends = [
            (tree.funding, root, Some(100)),
            (OutPoint::new(root, 0), middle, Some(101)),
        ];

        let refused = tree.validate(&spends, None).unwrap_err();

        assert!(refused.contains("is exiting"), "{refused}");
    }

    #[test]
    fn a_leaf_below_an_output_another_transaction_spent_is_refused() {
        let tree = Tree::new();
        let another = Txid::from_byte_array([6; 32]);

        let refused = tree
            .validate(&[(tree.funding, another, Some(100))], None)
            .unwrap_err();

        assert!(
            refused.contains("a conflicting transaction spends"),
            "{refused}"
        );
    }

    #[test]
    fn a_timelock_not_in_blocks_or_above_the_initial_timelock_is_refused() {
        for sequence in [
            Sequence::from_height(2001),
            Sequence::from_512_second_intervals(10),
            Sequence::MAX,
        ] {
            let mut tree = Tree::new();
            let leaf = &mut tree.pedigree.leaf;
            leaf.refund_tx = Some(refund(&leaf.node_tx, sequence));

            let refused = verify_pedigree(&tree.pedigree).unwrap_err().to_string();

            assert!(refused.contains("above the initial timelock"), "{refused}");
        }
    }

    #[test]
    fn a_refund_that_needs_renewal_passes() {
        for blocks in [100, 0] {
            let mut tree = Tree::new();
            let leaf = &mut tree.pedigree.leaf;
            leaf.refund_tx = Some(refund(&leaf.node_tx, Sequence::from_height(blocks)));

            assert!(verify_pedigree(&tree.pedigree).is_ok());
        }
    }

    #[test]
    fn a_leaf_without_a_refund_is_refused() {
        let mut tree = Tree::new();
        tree.pedigree.leaf.refund_tx = None;

        let refused = verify_pedigree(&tree.pedigree).unwrap_err().to_string();

        assert!(refused.contains("has no refund"), "{refused}");
    }

    #[test]
    fn only_a_key_path_signature_of_a_taproot_output_counts() {
        let tree = Tree::new();
        let root = &tree.root().node_tx;
        assert!(is_signed_for(root, &tree.funded));

        let mut other_script = tree.funded.clone();
        other_script.script_pubkey = ScriptBuf::new_op_return([1]);
        assert!(!is_signed_for(root, &other_script));
        let mut other_value = tree.funded.clone();
        other_value.value = Amount::from_sat(1);
        assert!(!is_signed_for(root, &other_value));
    }

    #[test]
    fn a_claimed_leaf_needs_a_root_the_ssp_checked_and_signatures_below_it() {
        let tree = Tree::new();
        let root = tree.root().node_tx.compute_txid();
        let all: HashSet<Txid> = std::iter::once(&tree.pedigree.leaf)
            .chain(&tree.pedigree.ancestors)
            .map(|node| node.node_tx.compute_txid())
            .collect();
        assert_eq!(verify_claimed(&tree.pedigree, &all), Ok(()));
        // With only the root stored, the SSP verifies each transaction below
        // the root against the output above it.
        assert_eq!(
            verify_claimed(&tree.pedigree, &HashSet::from([root])),
            Ok(())
        );

        let unchecked = verify_claimed(&tree.pedigree, &HashSet::new()).unwrap_err();
        assert!(
            unchecked.contains("has not checked the root"),
            "{unchecked}"
        );

        let forged = Tree::signed_with(7, [1, 8, 3]);
        let root = forged.root().node_tx.compute_txid();
        let unsigned = verify_claimed(&forged.pedigree, &HashSet::from([root])).unwrap_err();
        assert!(unsigned.contains("without a valid signature"), "{unsigned}");
    }

    #[derive(Default)]
    struct Coordinator {
        pedigrees: Mutex<Vec<LeafPedigree>>,
        queries: AtomicUsize,
        failing: AtomicBool,
    }

    #[async_trait::async_trait]
    impl Ancestors for Coordinator {
        async fn of(&self, leaves: &[TreeNode]) -> Result<Vec<LeafPedigree>, AncestorsError> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            if self.failing.load(Ordering::SeqCst) {
                let outage = OperatorRpcError::Transport("the coordinator is unavailable".into());
                return Err(AncestorsError::Query(outage));
            }
            let pedigrees = self.pedigrees.lock().unwrap();
            let reported: Vec<LeafPedigree> = leaves
                .iter()
                .filter_map(|leaf| {
                    pedigrees
                        .iter()
                        .find(|pedigree| pedigree.leaf.node_tx == leaf.node_tx)
                        .cloned()
                })
                .collect();
            if let Some(pedigree) = reported
                .iter()
                .find(|pedigree| !chain_reaches_root(&pedigree.leaf, &pedigree.ancestors))
            {
                return Err(AncestorsError::MissingAncestors(pedigree.leaf.id.clone()));
            }
            Ok(reported)
        }
    }

    struct Bitcoind {
        best_block: BlockHash,
        unspent: Mutex<HashMap<OutPoint, (TxOut, u64)>>,
        spends: Mutex<Vec<OutputSpend>>,
        calls: AtomicUsize,
        failing: AtomicBool,
        silent: AtomicBool,
    }

    impl Bitcoind {
        async fn called(&self) -> Result<(), ChainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.silent.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.failing.load(Ordering::SeqCst) {
                return Err(ChainError::General("bitcoind unavailable".into()));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl RequestChainClient for Bitcoind {
        async fn unspent_output(
            &self,
            outpoint: &OutPoint,
        ) -> Result<Option<UnspentOutput>, ChainError> {
            self.called().await?;
            Ok(self
                .unspent
                .lock()
                .unwrap()
                .get(outpoint)
                .map(|(tx_out, confirmations)| UnspentOutput {
                    tx_out: tx_out.clone(),
                    confirmations: *confirmations,
                    best_block: self.best_block,
                }))
        }

        async fn get_spends(&self, outpoints: &[OutPoint]) -> Result<Vec<OutputSpend>, ChainError> {
            self.called().await?;
            let spends = self.spends.lock().unwrap();
            Ok(spends
                .iter()
                .filter(|spend| outpoints.contains(&spend.outpoint))
                .cloned()
                .collect())
        }

        async fn send_to_address(
            &self,
            _address: &str,
            _amount_sats: u64,
        ) -> Result<String, ChainError> {
            unimplemented!()
        }
    }

    const TIP: u64 = 110;

    fn block(height: u64) -> BlockHeader {
        let mut hash = [0; 32];
        hash[..8].copy_from_slice(&height.to_be_bytes());
        BlockHeader {
            hash: BlockHash::from_byte_array(hash),
            height,
        }
    }

    struct Fixture {
        check: IncomingLeafCheck,
        coordinator: Arc<Coordinator>,
        bitcoind: Arc<Bitcoind>,
        pool: Arc<Pool>,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            let chain = Arc::new(ChainRepository::new(Arc::clone(&pool), Network::Regtest));
            for height in 100..=TIP {
                chain.add_block(&block(height), &[], &[]).await.unwrap();
            }
            let identity = key(1).public_key(&Secp256k1::new());
            let coordinator = Arc::new(Coordinator::default());
            let bitcoind = Arc::new(Bitcoind {
                best_block: block(TIP).hash,
                unspent: Mutex::default(),
                spends: Mutex::default(),
                calls: AtomicUsize::new(0),
                failing: AtomicBool::new(false),
                silent: AtomicBool::new(false),
            });
            let check = IncomingLeafCheck {
                leaf_store: Arc::new(PostgresLeafStore::new(
                    Arc::clone(&pool),
                    identity,
                    Arc::clone(&chain),
                )),
                chain_repository: chain,
                chain_tip: ChainTip::at(block(TIP)),
                ancestors: Arc::clone(&coordinator) as Arc<dyn Ancestors>,
                call_queue: Arc::new(CallQueue::new(
                    Arc::clone(&bitcoind) as Arc<dyn RequestChainClient>
                )),
                synced: Synced::at_tip(),
                confirmations: 3,
                timeout: Duration::from_secs(5),
            };
            Self {
                check,
                coordinator,
                bitcoind,
                pool,
                _database: database,
            }
        }

        async fn check(&self, leaf: &TreeNode) -> Result<(), String> {
            self.check
                .check_leaves(&[leaf])
                .await
                .map_err(|e| e.to_string())
        }

        fn calls(&self) -> (usize, usize) {
            (
                self.coordinator.queries.load(Ordering::SeqCst),
                self.bitcoind.calls.load(Ordering::SeqCst),
            )
        }

        fn reported_elsewhere(&self, tree: &Tree, pedigree: &LeafPedigree, confirmations: u64) {
            self.coordinator
                .pedigrees
                .lock()
                .unwrap()
                .push(pedigree.clone());
            self.bitcoind
                .unspent
                .lock()
                .unwrap()
                .insert(tree.funding, (tree.funded.clone(), confirmations));
        }

        async fn own(&self, tree: &Tree, height: u64) {
            let chain = &self.check.chain_repository;
            let address =
                Address::from_script(&tree.funded.script_pubkey, Network::Regtest).unwrap();
            chain
                .add_watch_addresses(std::slice::from_ref(&address))
                .await
                .unwrap();
            let funded = AddressUtxo {
                address,
                utxo: Txo {
                    block_height: height,
                    outpoint: tree.funding,
                    tx_out: tree.funded.clone(),
                },
            };
            chain
                .add_block(&block(height), &[funded], &[])
                .await
                .unwrap();
            let mut client = self.pool.get().await.unwrap();
            let db = client.transaction().await.unwrap();
            self.check
                .leaf_store
                .insert_leaves(&db, std::slice::from_ref(&tree.pedigree))
                .await
                .unwrap();
            db.commit().await.unwrap();
            self.record_unspent().await;
        }

        async fn record_unspent(&self) {
            let chain = &self.check.chain_repository;
            let queued = chain.next_unchecked_outputs(u32::MAX).await.unwrap();
            chain.record_checked_outputs(&queued, &[]).await.unwrap();
            chain.drop_unconfirmed_outputs().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_leaf_of_a_tree_the_ssp_has_stored_takes_no_call() {
        let mut fixture = Fixture::new().await;
        let tree = Tree::new();
        fixture.own(&tree, 105).await;

        assert_eq!(fixture.check(&tree.pedigree.leaf).await, Ok(()));
        assert_eq!(fixture.calls(), (0, 0));

        // Once the chain record has the leaf's parent as confirmed, the leaf is
        // exiting.
        let root = tree.root().node_tx.compute_txid();
        let middle = tree.middle().node_tx.compute_txid();
        let spends = [
            crate::chain::SpentTxo {
                outpoint: tree.funding,
                spending_tx: root,
                spending_input_index: 0,
            },
            crate::chain::SpentTxo {
                outpoint: OutPoint::new(root, 0),
                spending_tx: middle,
                spending_input_index: 0,
            },
        ];
        fixture
            .check
            .chain_repository
            .add_block(&block(TIP.saturating_add(1)), &[], &spends)
            .await
            .unwrap();
        fixture.check.chain_tip = ChainTip::at(block(TIP.saturating_add(1)));
        let refused = fixture.check(&tree.pedigree.leaf).await.unwrap_err();
        assert!(refused.contains("is exiting"), "{refused}");
        assert_eq!(fixture.calls(), (0, 0));
    }

    #[tokio::test]
    async fn a_funding_output_the_record_has_too_shallow_is_refused_without_a_call() {
        let fixture = Fixture::new().await;
        let tree = Tree::new();
        fixture.own(&tree, TIP.saturating_sub(1)).await;

        let refused = fixture.check(&tree.pedigree.leaf).await.unwrap_err();

        assert!(refused.contains("too few confirmations"), "{refused}");
        assert_eq!(fixture.calls(), (0, 0));
    }

    #[tokio::test]
    async fn a_renewed_leaf_of_a_tree_the_ssp_has_stored_takes_one_call_to_the_coordinator() {
        let fixture = Fixture::new().await;
        let tree = Tree::new();
        fixture.own(&tree, 105).await;
        let renewed = tree.renewed();
        fixture
            .coordinator
            .pedigrees
            .lock()
            .unwrap()
            .push(renewed.clone());

        assert_eq!(fixture.check(&renewed.leaf).await, Ok(()));
        assert_eq!(fixture.calls(), (1, 0));

        let node_txid = renewed.leaf.node_tx.compute_txid();
        let stored = fixture
            .check
            .leaf_store
            .stored_transactions(&[node_txid])
            .await
            .unwrap();
        assert!(stored.contains(&node_txid));
    }

    #[tokio::test]
    async fn a_leaf_of_a_tree_the_ssp_has_not_seen_takes_one_call_each_the_first_time() {
        let fixture = Fixture::new().await;
        let tree = Tree::new();
        fixture.reported_elsewhere(&tree, &tree.pedigree, 4);

        assert_eq!(fixture.check(&tree.pedigree.leaf).await, Ok(()));
        assert_eq!(fixture.calls(), (1, 1));

        // The SSP has stored the exit chain now. Until the spend check has
        // queried bitcoind for the outputs on that chain, the SSP queries
        // bitcoind on each check.
        assert_eq!(fixture.check(&tree.pedigree.leaf).await, Ok(()));
        assert_eq!(fixture.calls(), (1, 2));

        fixture.record_unspent().await;
        assert_eq!(fixture.check(&tree.pedigree.leaf).await, Ok(()));
        assert_eq!(fixture.calls(), (1, 2));
    }

    #[tokio::test]
    async fn a_tree_the_ssp_has_not_seen_is_validated_by_what_bitcoind_reports() {
        let fixture = Fixture::new().await;
        let shallow = Tree::signed_with(7, [1, 2, 3]);
        fixture.reported_elsewhere(&shallow, &shallow.pedigree, 2);
        let refused = fixture.check(&shallow.pedigree.leaf).await.unwrap_err();
        assert!(refused.contains("too few confirmations"), "{refused}");

        let forged = Tree::signed_with(8, [8, 2, 3]);
        fixture.reported_elsewhere(&forged, &forged.pedigree, 6);
        let refused = fixture.check(&forged.pedigree.leaf).await.unwrap_err();
        assert!(refused.contains("root has no valid signature"), "{refused}");

        let missing = Tree::signed_with(9, [1, 2, 3]);
        fixture
            .coordinator
            .pedigrees
            .lock()
            .unwrap()
            .push(missing.pedigree.clone());
        let refused = fixture.check(&missing.pedigree.leaf).await.unwrap_err();
        assert!(refused.contains("not on chain"), "{refused}");

        // The root of this tree is in a block, and the leaf's parent is in the
        // mempool.
        let exiting = Tree::signed_with(10, [1, 2, 3]);
        fixture
            .coordinator
            .pedigrees
            .lock()
            .unwrap()
            .push(exiting.pedigree.clone());
        let root = exiting.root().node_tx.compute_txid();
        let middle = exiting.middle().node_tx.compute_txid();
        fixture.bitcoind.spends.lock().unwrap().extend([
            OutputSpend {
                outpoint: exiting.funding,
                spender: root,
                block: Some(block(100)),
            },
            OutputSpend {
                outpoint: OutPoint::new(root, 0),
                spender: middle,
                block: None,
            },
        ]);
        let refused = fixture.check(&exiting.pedigree.leaf).await.unwrap_err();
        assert!(refused.contains("is exiting"), "{refused}");

        // The SSP stores nothing of a leaf it refuses.
        let refused_nodes: Vec<Txid> = [&shallow, &forged, &missing, &exiting]
            .iter()
            .map(|tree| tree.pedigree.leaf.node_tx.compute_txid())
            .collect();
        let stored = fixture
            .check
            .leaf_store
            .stored_transactions(&refused_nodes)
            .await
            .unwrap();
        assert!(stored.is_empty());
    }

    #[tokio::test]
    async fn a_leaf_the_coordinator_reports_without_its_root_is_refused() {
        let fixture = Fixture::new().await;
        let tree = Tree::new();
        let mut pedigree = tree.pedigree.clone();
        pedigree.ancestors.pop();
        fixture.reported_elsewhere(&tree, &pedigree, 4);
        let refused = fixture.check(&pedigree.leaf).await.unwrap_err();
        assert!(refused.contains("up to its tree's root"), "{refused}");
        assert_eq!(fixture.calls(), (1, 0));
    }

    #[tokio::test]
    async fn a_request_fails_when_the_ssp_cannot_check_the_leaves_and_the_next_one_goes_through() {
        let mut fixture = Fixture::new().await;
        fixture.check.timeout = Duration::from_millis(100);
        let tree = Tree::new();
        fixture.reported_elsewhere(&tree, &tree.pedigree, 4);
        let leaf = &tree.pedigree.leaf;
        let unavailable = IncomingLeafError::Unavailable.to_string();

        fixture.check.synced = Synced::new();
        assert_eq!(fixture.check(leaf).await, Err(unavailable.clone()));
        assert_eq!(fixture.calls(), (0, 0), "not synced");
        fixture.check.synced = Synced::at_tip();

        fixture.coordinator.failing.store(true, Ordering::SeqCst);
        assert_eq!(fixture.check(leaf).await, Err(unavailable.clone()));
        fixture.coordinator.failing.store(false, Ordering::SeqCst);

        fixture.bitcoind.failing.store(true, Ordering::SeqCst);
        assert_eq!(fixture.check(leaf).await, Err(unavailable.clone()));
        fixture.bitcoind.failing.store(false, Ordering::SeqCst);

        fixture.bitcoind.silent.store(true, Ordering::SeqCst);
        assert_eq!(fixture.check(leaf).await, Err(unavailable));
        fixture.bitcoind.silent.store(false, Ordering::SeqCst);

        let node_txid = leaf.node_tx.compute_txid();
        let stored = fixture
            .check
            .leaf_store
            .stored_transactions(&[node_txid])
            .await
            .unwrap();
        assert!(stored.is_empty(), "a failed check stores nothing");

        assert_eq!(fixture.check(leaf).await, Ok(()));
    }
}
