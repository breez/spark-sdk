use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::consensus::encode::serialize_hex;

use crate::{
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
    /// building all necessary transactions, and signing the CPFP fee-bump
    /// transactions using the provided signer.
    ///
    /// Uses a two-pass approach: first auto-selects leaves and builds CPFP
    /// chains assuming all ancestors are unconfirmed, then checks the chain
    /// service for already-confirmed ancestors. If any are found, rebuilds
    /// the CPFP chain skipping confirmed nodes.
    #[allow(clippy::too_many_lines)]
    pub async fn prepare_unilateral_exit(
        &self,
        request: PrepareUnilateralExitRequest,
        signer: Arc<dyn CpfpSigner>,
    ) -> Result<PrepareUnilateralExitResponse, SdkError> {
        let btc_network: bitcoin::Network = self.config.network.into();
        let destination = FromStr::from_str(&request.destination)
            .map_err(|e: bitcoin::address::ParseError| {
                SdkError::InvalidInput(format!("Invalid destination address: {e}"))
            })
            .and_then(
                |addr: bitcoin::Address<bitcoin::address::NetworkUnchecked>| {
                    addr.require_network(btc_network).map_err(|e| {
                        SdkError::InvalidInput(format!("Address network mismatch: {e}"))
                    })
                },
            )?;

        let inputs = request
            .inputs
            .into_iter()
            .map(|input| convert_cpfp_input(input, btc_network))
            .collect::<Result<Vec<_>, SdkError>>()?;

        // Pass 1: auto-select leaves and build CPFP chains assuming nothing is
        // confirmed.
        let exit_result = self
            .spark_wallet
            .unilateral_exit_autoselect(
                request.fee_rate_sat_per_vbyte,
                inputs.clone(),
                destination.clone(),
            )
            .await?;

        // Check the chain service for already-confirmed ancestors.
        let (confirmed_node_ids, unverified_node_ids) = check_ancestor_confirmations(
            &exit_result.leaf_tx_cpfp_psbts,
            self.chain_service.as_ref(),
        )
        .await;

        // Pass 2: if any ancestors are confirmed, rebuild the CPFP chain
        // skipping those nodes so the CPFP inputs are threaded correctly.
        // Reuse the prefetched nodes to ensure consistency between passes.
        let leaf_tx_cpfp_psbts = if confirmed_node_ids.is_empty() {
            exit_result.leaf_tx_cpfp_psbts
        } else {
            let selected_ids = exit_result
                .selected_leaves
                .iter()
                .map(|s| s.id.clone())
                .collect();
            self.spark_wallet
                .unilateral_exit(
                    request.fee_rate_sat_per_vbyte,
                    selected_ids,
                    inputs,
                    Some(exit_result.prefetched_nodes),
                    &confirmed_node_ids,
                )
                .await?
        };

        // Build a lookup for selected leaf metadata by leaf ID.
        let selected_by_id: HashMap<String, &spark_wallet::SelectedLeaf> = exit_result
            .selected_leaves
            .iter()
            .map(|s| (s.id.to_string(), s))
            .collect();

        // Sign CPFP PSBTs and group per leaf.
        let mut leaves = Vec::with_capacity(leaf_tx_cpfp_psbts.len());
        for leaf_psbts in leaf_tx_cpfp_psbts {
            let selected = selected_by_id
                .get(&leaf_psbts.leaf_id.to_string())
                .ok_or_else(|| {
                    SdkError::Generic(format!(
                        "Selected leaf metadata not found for {}",
                        leaf_psbts.leaf_id
                    ))
                })?;
            let mut transactions = Vec::with_capacity(leaf_psbts.tx_cpfp_psbts.len());
            for tc in leaf_psbts.tx_cpfp_psbts {
                let csv_timelock_blocks = tc.parent_tx.input.first().and_then(|input| {
                    let seq = input.sequence.to_consensus_u32();
                    // BIP68: bit 31 (disable flag) must be unset for relative lock
                    // Bit 22 unset means block-based (not time-based)
                    if seq & 0x8000_0000 == 0 && seq & 0x0040_0000 == 0 {
                        let blocks = seq & 0xFFFF;
                        if blocks > 0 { Some(blocks) } else { None }
                    } else {
                        None
                    }
                });

                // Finalize the ephemeral anchor input before passing to the signer
                let mut psbt = tc.child_psbt;
                for input in &mut psbt.inputs {
                    if let Some(ref tx_out) = input.witness_utxo
                        && tx_out.value.to_sat() == 0
                        && tx_out.script_pubkey.as_bytes() == [0x51, 0x02, 0x4e, 0x73]
                    {
                        input.final_script_witness = Some(bitcoin::Witness::new());
                    }
                }

                // Sign the PSBT via the external signer
                let psbt_bytes = psbt.serialize();
                let signed_psbt_bytes = signer
                    .sign_psbt(psbt_bytes)
                    .await
                    .map_err(|e| SdkError::Generic(format!("CPFP signer error: {e}")))?;
                let signed_psbt = bitcoin::Psbt::deserialize(&signed_psbt_bytes).map_err(|e| {
                    SdkError::Generic(format!("Failed to deserialize signed PSBT: {e}"))
                })?;

                // Extract the final signed transaction
                let signed_tx = signed_psbt.extract_tx_unchecked_fee_rate();

                transactions.push(UnilateralExitTransaction {
                    node_id: tc.node_id.to_string(),
                    tx_hex: serialize_hex(&tc.parent_tx),
                    cpfp_tx_hex: Some(serialize_hex(&signed_tx)),
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
            sweep_tx_hex: serialize_hex(&exit_result.sweep_tx),
            unverified_node_ids,
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

/// Walks each leaf's CPFP chain root-to-leaf and queries the chain service for
/// each ancestor's confirmation status. Stops walking a leaf at the first
/// unconfirmed ancestor since no descendant can be confirmed without it.
/// Shared ancestors — those that resolve to the same `TreeNodeId` across
/// leaves — are queried at most once.
///
/// Returns the set of confirmed ancestor node IDs and a list of node IDs
/// whose status could not be verified because the chain service returned an
/// error. The caller treats unverified nodes as unconfirmed but surfaces
/// them so the integrator can retry or diagnose connectivity issues.
async fn check_ancestor_confirmations(
    leaves: &[spark_wallet::LeafTxCpfpPsbts],
    chain_service: &dyn crate::chain::BitcoinChainService,
) -> (HashSet<spark_wallet::TreeNodeId>, Vec<String>) {
    let mut confirmed_node_ids: HashSet<spark_wallet::TreeNodeId> = HashSet::new();
    let mut known_unconfirmed: HashSet<spark_wallet::TreeNodeId> = HashSet::new();
    let mut unverified_node_ids: Vec<String> = Vec::new();

    for leaf_psbts in leaves {
        for tc in &leaf_psbts.tx_cpfp_psbts {
            if confirmed_node_ids.contains(&tc.node_id) {
                continue;
            }
            if known_unconfirmed.contains(&tc.node_id) {
                // Ancestor already seen unconfirmed in a prior leaf; no
                // descendant in this leaf can be confirmed without it.
                break;
            }
            let txid = tc.parent_tx.compute_txid().to_string();
            match chain_service.get_transaction_status(txid).await {
                Ok(status) if status.confirmed => {
                    confirmed_node_ids.insert(tc.node_id.clone());
                }
                Ok(_) => {
                    // Unconfirmed (possibly in mempool) — stop checking
                    // deeper nodes for this leaf since they can't be
                    // confirmed without their parent.
                    known_unconfirmed.insert(tc.node_id.clone());
                    break;
                }
                Err(_) => {
                    // Chain service error — assume unconfirmed but record
                    // that we couldn't verify.
                    known_unconfirmed.insert(tc.node_id.clone());
                    unverified_node_ids.push(tc.node_id.to_string());
                    break;
                }
            }
        }
    }

    (confirmed_node_ids, unverified_node_ids)
}

#[cfg(test)]
mod ancestor_confirmation_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use macros::async_test_all;

    use crate::chain::{BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo};
    use spark_wallet::{LeafTxCpfpPsbts, TreeNodeId, TxCpfpPsbt};

    /// Chain service stub with a pre-seeded map from txid → status and a
    /// query log for asserting call counts.
    struct MockChainService {
        statuses: HashMap<String, Result<TxStatus, ChainServiceError>>,
        queries: Mutex<Vec<String>>,
    }

    impl MockChainService {
        fn new() -> Self {
            Self {
                statuses: HashMap::new(),
                queries: Mutex::new(Vec::new()),
            }
        }

        fn confirmed(mut self, txid: &str) -> Self {
            self.statuses.insert(
                txid.to_string(),
                Ok(TxStatus {
                    confirmed: true,
                    block_height: Some(800_000),
                    block_time: Some(0),
                }),
            );
            self
        }

        fn error(mut self, txid: &str) -> Self {
            self.statuses.insert(
                txid.to_string(),
                Err(ChainServiceError::ServiceConnectivity("fail".into())),
            );
            self
        }

        fn query_count_for(&self, txid: &str) -> usize {
            self.queries
                .lock()
                .unwrap()
                .iter()
                .filter(|t| t.as_str() == txid)
                .count()
        }
    }

    #[macros::async_trait]
    impl BitcoinChainService for MockChainService {
        async fn get_transaction_status(
            &self,
            txid: String,
        ) -> Result<TxStatus, ChainServiceError> {
            self.queries.lock().unwrap().push(txid.clone());
            self.statuses.get(&txid).cloned().unwrap_or_else(|| {
                Ok(TxStatus {
                    confirmed: false,
                    block_height: None,
                    block_time: None,
                })
            })
        }
        async fn get_address_utxos(
            &self,
            _address: String,
        ) -> Result<Vec<Utxo>, ChainServiceError> {
            unreachable!("not used by pass-2")
        }
        async fn get_transaction_hex(&self, _txid: String) -> Result<String, ChainServiceError> {
            unreachable!("not used by pass-2")
        }
        async fn broadcast_transaction(&self, _tx: String) -> Result<(), ChainServiceError> {
            unreachable!("not used by pass-2")
        }
        async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
            unreachable!("not used by pass-2")
        }
    }

    fn dummy_tx(unique_tag: u32) -> Transaction {
        // Varying locktime so each tx has a unique txid.
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::from_consensus(unique_tag),
            input: vec![],
            output: vec![],
        }
    }

    fn tx_cpfp(node_id: &str, tx: Transaction) -> TxCpfpPsbt {
        let psbt = bitcoin::Psbt::from_unsigned_tx(tx.clone()).unwrap();
        TxCpfpPsbt {
            node_id: TreeNodeId::from_str(node_id).unwrap(),
            parent_tx: tx,
            child_psbt: psbt,
        }
    }

    fn leaf(leaf_id: &str, entries: Vec<TxCpfpPsbt>) -> LeafTxCpfpPsbts {
        LeafTxCpfpPsbts {
            leaf_id: TreeNodeId::from_str(leaf_id).unwrap(),
            tx_cpfp_psbts: entries,
        }
    }

    #[async_test_all]
    async fn test_stops_at_first_unconfirmed() {
        // All ancestors unconfirmed → the walk stops at the first one per leaf.
        let root_tx = dummy_tx(1);
        let leaf_tx = dummy_tx(2);
        let leaves = vec![leaf(
            "leaf",
            vec![tx_cpfp("root", root_tx), tx_cpfp("leaf-node", leaf_tx)],
        )];
        let chain = MockChainService::new();

        let (confirmed, unverified) = check_ancestor_confirmations(&leaves, &chain).await;

        assert!(confirmed.is_empty());
        assert!(unverified.is_empty());
        assert_eq!(
            chain.queries.lock().unwrap().len(),
            1,
            "walk must stop at the first unconfirmed ancestor"
        );
    }

    #[async_test_all]
    async fn test_confirmed_ancestor_is_recorded() {
        let root_tx = dummy_tx(1);
        let leaf_tx = dummy_tx(2);
        let root_txid = root_tx.compute_txid().to_string();
        let leaves = vec![leaf(
            "leaf",
            vec![tx_cpfp("root", root_tx), tx_cpfp("leaf-node", leaf_tx)],
        )];
        let chain = MockChainService::new().confirmed(&root_txid);

        let (confirmed, unverified) = check_ancestor_confirmations(&leaves, &chain).await;

        assert_eq!(confirmed.len(), 1);
        assert!(confirmed.contains(&TreeNodeId::from_str("root").unwrap()));
        assert!(unverified.is_empty());
    }

    #[async_test_all]
    async fn test_chain_service_error_records_unverified() {
        let root_tx = dummy_tx(1);
        let root_txid = root_tx.compute_txid().to_string();
        let leaves = vec![leaf("leaf", vec![tx_cpfp("root", root_tx)])];
        let chain = MockChainService::new().error(&root_txid);

        let (confirmed, unverified) = check_ancestor_confirmations(&leaves, &chain).await;

        assert!(confirmed.is_empty());
        assert_eq!(unverified, vec!["root".to_string()]);
    }

    #[async_test_all]
    async fn test_shared_confirmed_ancestor_queried_once() {
        // Two leaves descending from the same root ancestor. With the root
        // marked confirmed, it must only be queried once even though both
        // leaves include it.
        let root_tx = dummy_tx(1);
        let leaf_a_tx = dummy_tx(2);
        let leaf_b_tx = dummy_tx(3);
        let root_txid = root_tx.compute_txid().to_string();

        let leaves = vec![
            leaf(
                "leaf-a",
                vec![
                    tx_cpfp("root", root_tx.clone()),
                    tx_cpfp("leaf-a-node", leaf_a_tx),
                ],
            ),
            leaf(
                "leaf-b",
                vec![tx_cpfp("root", root_tx), tx_cpfp("leaf-b-node", leaf_b_tx)],
            ),
        ];
        let chain = MockChainService::new().confirmed(&root_txid);

        let (confirmed, unverified) = check_ancestor_confirmations(&leaves, &chain).await;

        assert_eq!(confirmed.len(), 1);
        assert!(unverified.is_empty());
        assert_eq!(
            chain.query_count_for(&root_txid),
            1,
            "shared confirmed ancestor must be queried at most once"
        );
    }

    #[async_test_all]
    async fn test_shared_unconfirmed_ancestor_queried_once() {
        // Two leaves sharing an unconfirmed root. The known_unconfirmed cache
        // must prevent the second leaf from re-querying the same ancestor.
        let root_tx = dummy_tx(1);
        let leaf_a_tx = dummy_tx(2);
        let leaf_b_tx = dummy_tx(3);
        let root_txid = root_tx.compute_txid().to_string();

        let leaves = vec![
            leaf(
                "leaf-a",
                vec![
                    tx_cpfp("root", root_tx.clone()),
                    tx_cpfp("leaf-a-node", leaf_a_tx),
                ],
            ),
            leaf(
                "leaf-b",
                vec![tx_cpfp("root", root_tx), tx_cpfp("leaf-b-node", leaf_b_tx)],
            ),
        ];
        let chain = MockChainService::new();

        let (confirmed, unverified) = check_ancestor_confirmations(&leaves, &chain).await;

        assert!(confirmed.is_empty());
        assert!(unverified.is_empty());
        assert_eq!(
            chain.query_count_for(&root_txid),
            1,
            "shared unconfirmed ancestor must be cached and not re-queried"
        );
    }
}
