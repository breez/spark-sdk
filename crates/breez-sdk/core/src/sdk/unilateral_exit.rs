use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::Address;
use bitcoin::address::NetworkUnchecked;
use bitcoin::consensus::encode::{deserialize_hex, serialize_hex};

use crate::{
    chain::{BitcoinChainService, Outspend},
    error::SdkError,
    models::{
        PrepareUnilateralExitRequest, PrepareUnilateralExitResponse, UnilateralExitCpfpInput,
        UnilateralExitLeaf, UnilateralExitTransaction,
    },
    signer::CpfpSigner,
};

use super::BreezSdk;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Prepares a unilateral exit by automatically selecting profitable leaves,
    /// discovering which (if any) outputs along the exit path have already been
    /// spent, and signing the CPFP fee-bump transactions that still need to be
    /// broadcast.
    ///
    /// The flow:
    ///   1. Build the optimistic exit path assuming nothing is spent on-chain.
    ///   2. Walk each leaf's entries, asking the chain service — per output —
    ///      whether it's been spent and by which transaction. Hydrate known
    ///      spenders from the tree; fetch unknowns by txid.
    ///   3. Hand the collected confirmed-spender transactions to spark-wallet,
    ///      which rebuilds the CPFP list (with `child_psbt: None` for already-
    ///      spent steps) and the sweep against whichever refund variant
    ///      actually landed.
    #[allow(clippy::too_many_lines)]
    pub async fn prepare_unilateral_exit(
        &self,
        request: PrepareUnilateralExitRequest,
        signer: Arc<dyn CpfpSigner>,
    ) -> Result<PrepareUnilateralExitResponse, SdkError> {
        let btc_network: bitcoin::Network = self.config.network.into();
        let destination = request
            .destination
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|e| SdkError::InvalidInput(format!("Invalid destination address: {e}")))?
            .require_network(btc_network)
            .map_err(|e| SdkError::InvalidInput(format!("Address network mismatch: {e}")))?;

        let inputs = request
            .inputs
            .into_iter()
            .map(|input| convert_cpfp_input(input, btc_network))
            .collect::<Result<Vec<_>, SdkError>>()?;

        // Pass 1: optimistic exit path — every CPFP entry has `child_psbt: Some`.
        let exit_result = self
            .spark_wallet
            .unilateral_exit_autoselect(
                request.fee_rate_sat_per_vbyte,
                inputs.clone(),
                destination.clone(),
            )
            .await?;

        // Walk the output tree to discover which (if any) steps are already
        // spent on-chain, and by which transactions.
        let (confirmed_spenders, unverified_node_ids) = discover_confirmed_spenders(
            &exit_result.leaf_tx_cpfp_psbts,
            self.chain_service.as_ref(),
        )
        .await;

        // Pass 2: feed the confirmed spenders back to spark-wallet. It rebuilds
        // the CPFP list (setting `child_psbt: None` for consumed steps) and
        // constructs the sweep pointing at whichever refund variant credited
        // each leaf's P2TR.
        let selected_ids = exit_result
            .selected_leaves
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let final_result = self
            .spark_wallet
            .unilateral_exit(
                request.fee_rate_sat_per_vbyte,
                selected_ids,
                inputs,
                destination,
                Some(exit_result.prefetched_nodes),
                confirmed_spenders,
            )
            .await?;

        // Build a lookup for selected leaf metadata by leaf ID.
        let selected_by_id: HashMap<&spark_wallet::TreeNodeId, &spark_wallet::SelectedLeaf> =
            exit_result
                .selected_leaves
                .iter()
                .map(|s| (&s.id, s))
                .collect();

        // Sign CPFP PSBTs and group per leaf. Entries with `child_psbt: None`
        // translate to `cpfp_tx_hex: None` on the public response — the
        // integrator is expected to skip these.
        let mut leaves = Vec::with_capacity(final_result.leaves.len());
        for leaf_psbts in final_result.leaves {
            let selected = selected_by_id.get(&leaf_psbts.leaf_id).ok_or_else(|| {
                SdkError::Generic(format!(
                    "Selected leaf metadata not found for {}",
                    leaf_psbts.leaf_id
                ))
            })?;
            let mut transactions = Vec::with_capacity(leaf_psbts.tx_cpfp_psbts.len());
            for tc in leaf_psbts.tx_cpfp_psbts {
                // BIP68 requires *every* input's relative lock to be satisfied
                // before the tx can land, so the effective wait is the max
                // block-based CSV across all inputs. Today's CPFP variants
                // (node_tx, refund_tx) are single-input so input[0] was enough,
                // but spark already builds multi-input spark txs elsewhere
                // (e.g. connector_refund_tx) and a future variant could land
                // here as parent_tx.
                let csv_timelock_blocks = tc
                    .parent_tx
                    .input
                    .iter()
                    .filter_map(|input| match input.sequence.to_relative_lock_time()? {
                        bitcoin::relative::LockTime::Blocks(h) => {
                            let v = u32::from(h.value());
                            if v > 0 { Some(v) } else { None }
                        }
                        bitcoin::relative::LockTime::Time(_) => None,
                    })
                    .max();

                let cpfp_tx_hex = match tc.child_psbt {
                    None => None,
                    Some(mut psbt) => {
                        // Finalize the ephemeral anchor input before passing to the signer
                        for input in &mut psbt.inputs {
                            if let Some(ref tx_out) = input.witness_utxo
                                && spark_wallet::is_ephemeral_anchor_output(tx_out)
                            {
                                input.final_script_witness = Some(bitcoin::Witness::new());
                            }
                        }

                        let psbt_bytes = psbt.serialize();
                        let signed_psbt_bytes = signer
                            .sign_psbt(psbt_bytes)
                            .await
                            .map_err(|e| SdkError::Generic(format!("CPFP signer error: {e}")))?;
                        let signed_psbt =
                            bitcoin::Psbt::deserialize(&signed_psbt_bytes).map_err(|e| {
                                SdkError::Generic(format!("Failed to deserialize signed PSBT: {e}"))
                            })?;
                        let signed_tx = signed_psbt.extract_tx_unchecked_fee_rate();
                        Some(serialize_hex(&signed_tx))
                    }
                };

                transactions.push(UnilateralExitTransaction {
                    node_id: tc.node_id.to_string(),
                    tx_hex: serialize_hex(&tc.parent_tx),
                    cpfp_tx_hex,
                    csv_timelock_blocks,
                });
            }
            leaves.push(UnilateralExitLeaf {
                leaf_id: leaf_psbts.leaf_id.to_string(),
                value: selected.value,
                estimated_cost: selected.estimated_cost,
                transactions,
            });
        }

        Ok(PrepareUnilateralExitResponse {
            leaves,
            sweep_tx_hex: serialize_hex(&final_result.sweep_tx),
            unverified_node_ids: unverified_node_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
        })
    }
}

/// Converts a public-API [`UnilateralExitCpfpInput`] to the internal [`spark_wallet::CpfpInput`].
fn convert_cpfp_input(
    input: UnilateralExitCpfpInput,
    network: bitcoin::Network,
) -> Result<spark_wallet::CpfpInput, SdkError> {
    match input {
        UnilateralExitCpfpInput::P2wpkh {
            txid,
            vout,
            value,
            pubkey,
        } => {
            let txid = bitcoin::Txid::from_str(&txid)
                .map_err(|e| SdkError::Generic(format!("Invalid txid: {e}")))?;
            let pubkey_bytes = hex::decode(&pubkey)
                .map_err(|e| SdkError::Generic(format!("Invalid pubkey hex: {e}")))?;
            let pubkey = bitcoin::secp256k1::PublicKey::from_slice(&pubkey_bytes)
                .map_err(|e| SdkError::Generic(format!("Invalid pubkey: {e}")))?;
            let script_pubkey =
                bitcoin::Address::p2wpkh(&bitcoin::CompressedPublicKey(pubkey), network)
                    .script_pubkey();
            Ok(spark_wallet::CpfpInput {
                outpoint: bitcoin::OutPoint { txid, vout },
                witness_utxo: bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(value),
                    script_pubkey,
                },
                signed_input_weight: 272,
            })
        }
        UnilateralExitCpfpInput::P2tr {
            txid,
            vout,
            value,
            pubkey,
        } => {
            let txid = bitcoin::Txid::from_str(&txid)
                .map_err(|e| SdkError::Generic(format!("Invalid txid: {e}")))?;
            let pubkey_bytes = hex::decode(&pubkey)
                .map_err(|e| SdkError::Generic(format!("Invalid pubkey hex: {e}")))?;
            let pubkey = bitcoin::secp256k1::PublicKey::from_slice(&pubkey_bytes)
                .map_err(|e| SdkError::Generic(format!("Invalid pubkey: {e}")))?;
            let secp = bitcoin::key::Secp256k1::new();
            let (xonly, _) = pubkey.x_only_public_key();
            let script_pubkey = bitcoin::Address::p2tr(&secp, xonly, None, network).script_pubkey();
            Ok(spark_wallet::CpfpInput {
                outpoint: bitcoin::OutPoint { txid, vout },
                witness_utxo: bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(value),
                    script_pubkey,
                },
                signed_input_weight: 230,
            })
        }
        UnilateralExitCpfpInput::Custom {
            txid,
            vout,
            value,
            script_pubkey_hex,
            signed_input_weight,
        } => {
            let txid = bitcoin::Txid::from_str(&txid)
                .map_err(|e| SdkError::Generic(format!("Invalid txid: {e}")))?;
            let script_bytes = hex::decode(&script_pubkey_hex)
                .map_err(|e| SdkError::Generic(format!("Invalid scriptPubKey hex: {e}")))?;
            let script_pubkey = bitcoin::ScriptBuf::from(script_bytes);
            Ok(spark_wallet::CpfpInput {
                outpoint: bitcoin::OutPoint { txid, vout },
                witness_utxo: bitcoin::TxOut {
                    value: bitcoin::Amount::from_sat(value),
                    script_pubkey,
                },
                signed_input_weight,
            })
        }
    }
}

/// Walks each leaf's exit path top-down and asks the chain service whether the
/// tracked output(s) for each step have been spent. When a step's output has
/// been spent by a confirmed transaction, the spender is added to the returned
/// list — resolved from the entry's hydrated `known_spenders` when possible,
/// otherwise fetched via `get_transaction_hex`. The walk short-circuits as
/// soon as a step is not yet done: descendants cannot be confirmed.
///
/// Returns the deduplicated confirmed-spender transactions plus the node IDs
/// whose status could not be verified because the chain service returned an
/// error for that step.
#[doc(hidden)]
#[allow(clippy::single_match_else, clippy::map_entry)]
pub async fn discover_confirmed_spenders(
    leaves: &[spark_wallet::LeafTxCpfpPsbts],
    chain_service: &dyn BitcoinChainService,
) -> (Vec<bitcoin::Transaction>, Vec<spark_wallet::TreeNodeId>) {
    let mut outspend_cache: HashMap<bitcoin::OutPoint, Option<Outspend>> = HashMap::new();
    let mut confirmed: HashMap<bitcoin::Txid, bitcoin::Transaction> = HashMap::new();
    let mut unverified_node_ids: Vec<spark_wallet::TreeNodeId> = Vec::new();

    'leaves: for leaf_psbts in leaves {
        for tc in &leaf_psbts.tx_cpfp_psbts {
            // For this step, check each candidate outpoint for a confirmed
            // spender. First hit wins; an error on any candidate counts as
            // unverified for the whole step.
            let mut step_resolved = false;
            for outpoint in &tc.spent_outpoints {
                let cached = outspend_cache.get(outpoint).cloned();
                let outspend = match cached {
                    Some(Some(o)) => o,
                    Some(None) => {
                        // Previously errored on this outpoint — treat as
                        // unverified once per step.
                        if !unverified_node_ids.contains(&tc.node_id) {
                            unverified_node_ids.push(tc.node_id.clone());
                        }
                        continue 'leaves;
                    }
                    None => match chain_service
                        .get_outspend(outpoint.txid.to_string(), outpoint.vout)
                        .await
                    {
                        Ok(o) => {
                            outspend_cache.insert(*outpoint, Some(o.clone()));
                            o
                        }
                        Err(_) => {
                            outspend_cache.insert(*outpoint, None);
                            unverified_node_ids.push(tc.node_id.clone());
                            continue 'leaves;
                        }
                    },
                };

                let Some(status) = outspend.status.as_ref() else {
                    continue;
                };
                if !outspend.spent || !status.confirmed {
                    continue;
                }
                let Some(spender_txid_str) = outspend.txid.as_ref() else {
                    continue;
                };
                let Ok(spender_txid) = bitcoin::Txid::from_str(spender_txid_str) else {
                    continue;
                };

                // Resolve the spender transaction. Check hydrated sources first.
                if !confirmed.contains_key(&spender_txid) {
                    let resolved = if tc.parent_tx.compute_txid() == spender_txid {
                        Some(tc.parent_tx.clone())
                    } else if let Some(tx) = tc
                        .known_spenders
                        .iter()
                        .find(|t| t.compute_txid() == spender_txid)
                    {
                        Some(tx.clone())
                    } else {
                        // Unknown spender — a newer version of the Spark
                        // software signed a spending transaction with our
                        // keys. Fetch it so spark-wallet can inspect its
                        // outputs for the leaf P2TR.
                        match chain_service
                            .get_transaction_hex(spender_txid_str.clone())
                            .await
                        {
                            Ok(hex) => deserialize_hex::<bitcoin::Transaction>(&hex).ok(),
                            Err(_) => {
                                unverified_node_ids.push(tc.node_id.clone());
                                continue 'leaves;
                            }
                        }
                    };
                    let Some(tx) = resolved else {
                        unverified_node_ids.push(tc.node_id.clone());
                        continue 'leaves;
                    };
                    confirmed.insert(spender_txid, tx);
                }

                step_resolved = true;
                break;
            }

            if !step_resolved {
                // No candidate outpoint is confirmed-spent. Descendants can't
                // be confirmed either; stop walking this leaf.
                continue 'leaves;
            }
        }
    }

    // Dedup is via the HashMap keyed on txid; also dedup unverified node ids.
    let mut seen = HashSet::new();
    unverified_node_ids.retain(|n| seen.insert(n.clone()));

    (confirmed.into_values().collect(), unverified_node_ids)
}

#[cfg(test)]
mod discover_confirmed_spenders_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use bitcoin::{
        OutPoint, Sequence, Transaction, TxIn, TxOut, absolute::LockTime, transaction::Version,
    };
    use macros::async_test_all;

    use crate::chain::{BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo};
    use spark_wallet::{LeafTxCpfpPsbts, TreeNodeId, TxCpfpPsbt};

    /// Chain service stub that returns seeded outspend answers and transaction
    /// hex by txid. Counts queries for cache-dedup assertions.
    struct MockChainService {
        outspends: HashMap<(String, u32), Result<Outspend, ChainServiceError>>,
        tx_hex: HashMap<String, Result<String, ChainServiceError>>,
        outspend_queries: Mutex<Vec<(String, u32)>>,
        tx_hex_queries: Mutex<Vec<String>>,
    }

    impl MockChainService {
        fn new() -> Self {
            Self {
                outspends: HashMap::new(),
                tx_hex: HashMap::new(),
                outspend_queries: Mutex::new(Vec::new()),
                tx_hex_queries: Mutex::new(Vec::new()),
            }
        }

        fn unspent(mut self, op: OutPoint) -> Self {
            self.outspends.insert(
                (op.txid.to_string(), op.vout),
                Ok(Outspend {
                    spent: false,
                    txid: None,
                    vin: None,
                    status: None,
                }),
            );
            self
        }

        fn spent_confirmed(mut self, op: OutPoint, spender_txid: bitcoin::Txid) -> Self {
            self.outspends.insert(
                (op.txid.to_string(), op.vout),
                Ok(Outspend {
                    spent: true,
                    txid: Some(spender_txid.to_string()),
                    vin: Some(0),
                    status: Some(TxStatus {
                        confirmed: true,
                        block_height: Some(800_000),
                        block_time: Some(0),
                    }),
                }),
            );
            self
        }

        fn spent_mempool(mut self, op: OutPoint, spender_txid: bitcoin::Txid) -> Self {
            self.outspends.insert(
                (op.txid.to_string(), op.vout),
                Ok(Outspend {
                    spent: true,
                    txid: Some(spender_txid.to_string()),
                    vin: Some(0),
                    status: Some(TxStatus {
                        confirmed: false,
                        block_height: None,
                        block_time: None,
                    }),
                }),
            );
            self
        }

        fn outspend_error(mut self, op: OutPoint) -> Self {
            self.outspends.insert(
                (op.txid.to_string(), op.vout),
                Err(ChainServiceError::ServiceConnectivity("fail".into())),
            );
            self
        }

        fn tx_hex(mut self, tx: &Transaction) -> Self {
            self.tx_hex
                .insert(tx.compute_txid().to_string(), Ok(serialize_hex(tx)));
            self
        }

        fn outspend_query_count(&self, op: OutPoint) -> usize {
            self.outspend_queries
                .lock()
                .unwrap()
                .iter()
                .filter(|(txid, vout)| *txid == op.txid.to_string() && *vout == op.vout)
                .count()
        }
    }

    #[macros::async_trait]
    impl BitcoinChainService for MockChainService {
        async fn get_transaction_status(
            &self,
            _txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            unreachable!("walk uses get_outspend, not get_transaction_status")
        }
        async fn get_address_utxos(
            &self,
            _address: String,
        ) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!("walk does not query addresses")
        }
        async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
            self.tx_hex_queries.lock().unwrap().push(txid.clone());
            self.tx_hex
                .get(&txid)
                .cloned()
                .unwrap_or_else(|| Err(ChainServiceError::Generic("not seeded".into())))
        }
        async fn get_outspend(
            &self,
            txid: String,
            vout: u32,
        ) -> Result<Outspend, ChainServiceError> {
            self.outspend_queries
                .lock()
                .unwrap()
                .push((txid.clone(), vout));
            self.outspends
                .get(&(txid, vout))
                .cloned()
                .unwrap_or_else(|| {
                    Ok(Outspend {
                        spent: false,
                        txid: None,
                        vin: None,
                        status: None,
                    })
                })
        }
        async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
            unreachable!("walk does not broadcast")
        }
        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            unreachable!("walk does not query fees")
        }
    }

    fn dummy_tx(unique_tag: u32, prev: OutPoint) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_consensus(unique_tag),
            input: vec![TxIn {
                previous_output: prev,
                sequence: Sequence::MAX,
                ..Default::default()
            }],
            output: vec![TxOut {
                value: bitcoin::Amount::from_sat(1_000),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        }
    }

    fn op(tx: &Transaction, vout: u32) -> OutPoint {
        OutPoint {
            txid: tx.compute_txid(),
            vout,
        }
    }

    fn tx_entry(
        node_id: &str,
        parent_tx: Transaction,
        spent_outpoints: Vec<OutPoint>,
        known_spenders: Vec<Transaction>,
    ) -> TxCpfpPsbt {
        let psbt = bitcoin::Psbt::from_unsigned_tx(parent_tx.clone()).unwrap();
        TxCpfpPsbt {
            node_id: TreeNodeId::from_str(node_id).unwrap(),
            parent_tx,
            child_psbt: Some(psbt),
            spent_outpoints,
            known_spenders,
        }
    }

    fn leaf(leaf_id: &str, entries: Vec<TxCpfpPsbt>) -> LeafTxCpfpPsbts {
        LeafTxCpfpPsbts {
            leaf_id: TreeNodeId::from_str(leaf_id).unwrap(),
            tx_cpfp_psbts: entries,
        }
    }

    fn deposit_outpoint() -> OutPoint {
        OutPoint {
            txid: bitcoin::Txid::from_str(
                "1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap(),
            vout: 0,
        }
    }

    #[async_test_all]
    async fn test_cpfp_variant_confirmed_no_fetch() {
        let deposit = deposit_outpoint();
        let node_tx = dummy_tx(1, deposit);
        let entries = vec![tx_entry("root", node_tx.clone(), vec![deposit], vec![])];
        let leaves = vec![leaf("root", entries)];

        let chain = MockChainService::new().spent_confirmed(deposit, node_tx.compute_txid());

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0].compute_txid(), node_tx.compute_txid());
        assert!(unverified.is_empty());
        assert!(
            chain.tx_hex_queries.lock().unwrap().is_empty(),
            "CPFP variant match should not require a fetch"
        );
    }

    #[async_test_all]
    async fn test_known_non_cpfp_variant_confirmed_no_fetch() {
        // Simulates direct_tx landing at the node step. Known via known_spenders.
        let deposit = deposit_outpoint();
        let node_tx = dummy_tx(1, deposit);
        let direct_tx = dummy_tx(99, deposit);
        let entries = vec![tx_entry(
            "root",
            node_tx.clone(),
            vec![deposit],
            vec![direct_tx.clone()],
        )];
        let leaves = vec![leaf("root", entries)];

        let chain = MockChainService::new().spent_confirmed(deposit, direct_tx.compute_txid());

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0].compute_txid(), direct_tx.compute_txid());
        assert!(unverified.is_empty());
        assert!(
            chain.tx_hex_queries.lock().unwrap().is_empty(),
            "known_spenders hit should not require a fetch"
        );
    }

    #[async_test_all]
    async fn test_outspend_unspent_short_circuits_leaf() {
        let deposit = deposit_outpoint();
        let node_tx = dummy_tx(1, deposit);
        let refund_tx = dummy_tx(2, op(&node_tx, 0));
        let entries = vec![
            tx_entry("root", node_tx.clone(), vec![deposit], vec![]),
            tx_entry("root", refund_tx, vec![op(&node_tx, 0)], vec![]),
        ];
        let leaves = vec![leaf("root", entries)];

        let chain = MockChainService::new().unspent(deposit);

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert!(confirmed.is_empty());
        assert!(unverified.is_empty());
        assert_eq!(
            chain.outspend_query_count(op(&node_tx, 0)),
            0,
            "refund outpoint must not be queried once the node step is unconfirmed",
        );
    }

    #[async_test_all]
    async fn test_mempool_only_spender_treated_as_unconfirmed() {
        let deposit = deposit_outpoint();
        let node_tx = dummy_tx(1, deposit);
        let entries = vec![tx_entry("root", node_tx.clone(), vec![deposit], vec![])];
        let leaves = vec![leaf("root", entries)];

        let chain = MockChainService::new().spent_mempool(deposit, node_tx.compute_txid());

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert!(confirmed.is_empty());
        assert!(unverified.is_empty());
    }

    #[async_test_all]
    async fn test_chain_error_records_unverified() {
        let deposit = deposit_outpoint();
        let node_tx = dummy_tx(1, deposit);
        let entries = vec![tx_entry("root", node_tx, vec![deposit], vec![])];
        let leaves = vec![leaf("root", entries)];

        let chain = MockChainService::new().outspend_error(deposit);

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert!(confirmed.is_empty());
        assert_eq!(unverified, vec![TreeNodeId::from_str("root").unwrap()]);
    }

    #[async_test_all]
    async fn test_shared_ancestor_queried_once() {
        let deposit = deposit_outpoint();
        let node_tx = dummy_tx(1, deposit);

        let leaves = vec![
            leaf(
                "a",
                vec![tx_entry("root", node_tx.clone(), vec![deposit], vec![])],
            ),
            leaf(
                "b",
                vec![tx_entry("root", node_tx.clone(), vec![deposit], vec![])],
            ),
        ];

        let chain = MockChainService::new().spent_confirmed(deposit, node_tx.compute_txid());

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert_eq!(confirmed.len(), 1);
        assert!(unverified.is_empty());
        assert_eq!(
            chain.outspend_query_count(deposit),
            1,
            "shared outpoint must be cached across leaves",
        );
    }

    /// Our outpoint at a non-zero input index of a multi-input spender (as in
    /// `create_connector_refund_txs` and future protocol additions) must still
    /// resolve. The walk itself only checks the spender txid, so this test
    /// verifies the returned Vec contains the multi-input tx even when our
    /// outpoint is at input[1].
    #[async_test_all]
    async fn test_multi_input_spender_matches_at_nonzero_index() {
        let deposit = deposit_outpoint();
        let connector = OutPoint {
            txid: bitcoin::Txid::from_str(
                "2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
            vout: 1,
        };
        let node_tx = dummy_tx(1, deposit);
        // A multi-input spender: input[0] = connector, input[1] = our outpoint.
        let spender = Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_consensus(42),
            input: vec![
                TxIn {
                    previous_output: connector,
                    sequence: Sequence::MAX,
                    ..Default::default()
                },
                TxIn {
                    previous_output: deposit,
                    sequence: Sequence::MAX,
                    ..Default::default()
                },
            ],
            output: vec![TxOut {
                value: bitcoin::Amount::from_sat(500),
                script_pubkey: bitcoin::ScriptBuf::new(),
            }],
        };

        // Unknown to us: force a fetch via tx_hex.
        let entries = vec![tx_entry("root", node_tx, vec![deposit], vec![])];
        let leaves = vec![leaf("root", entries)];
        let chain = MockChainService::new()
            .spent_confirmed(deposit, spender.compute_txid())
            .tx_hex(&spender);

        let (confirmed, unverified) = discover_confirmed_spenders(&leaves, &chain).await;

        assert_eq!(
            confirmed.len(),
            1,
            "multi-input unknown spender must be returned"
        );
        assert_eq!(confirmed[0].compute_txid(), spender.compute_txid());
        assert!(unverified.is_empty());
        assert_eq!(
            chain.tx_hex_queries.lock().unwrap().len(),
            1,
            "unknown spender must be fetched once"
        );
    }
}
