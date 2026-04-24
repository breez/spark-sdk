use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bitcoin::{
    Amount, OutPoint, Psbt, Transaction, TxIn, TxOut, absolute::LockTime, psbt,
    transaction::Version,
};
use tracing::trace;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            SparkRpcClient,
            spark::{QueryNodesRequest, TreeNodeIds, query_nodes_request::Source},
        },
    },
    services::ServiceError,
    tree::{TreeNode, TreeNodeId},
    utils::{
        paging::{PagingFilter, PagingResult, pager},
        transactions::is_ephemeral_anchor_output,
    },
};

/// A UTXO input for CPFP fee-bumping.
///
/// The caller provides the full `witness_utxo` (value + scriptPubKey) and the expected
/// signed input weight. Change outputs reuse `witness_utxo.script_pubkey`.
pub struct CpfpInput {
    pub outpoint: OutPoint,
    pub witness_utxo: TxOut,
    pub signed_input_weight: u64,
}

pub struct TxCpfpPsbt {
    pub parent_tx: Transaction,
    pub child_psbt: Psbt,
}

pub struct LeafTxCpfpPsbts {
    pub leaf_id: TreeNodeId,
    pub tx_cpfp_psbts: Vec<TxCpfpPsbt>,
}

pub struct UnilateralExitService {
    operator_pool: Arc<OperatorPool>,
    network: Network,
}

impl UnilateralExitService {
    pub fn new(operator_pool: Arc<OperatorPool>, network: Network) -> Self {
        UnilateralExitService {
            operator_pool,
            network,
        }
    }

    pub async fn unilateral_exit(
        &self,
        fee_rate: u64,
        leaf_ids: Vec<TreeNodeId>,
        mut inputs: Vec<CpfpInput>,
    ) -> Result<Vec<LeafTxCpfpPsbts>, ServiceError> {
        if leaf_ids.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one leaf ID is required".to_string(),
            ));
        }
        if inputs.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one CPFP input is required".to_string(),
            ));
        }

        let mut all_leaf_tx_cpfp_psbts = Vec::new();
        let mut checked_txs = HashSet::new();

        // Fetch leaves and parents for the given leaf IDs
        let tree_nodes: HashMap<TreeNodeId, TreeNode> = self
            .fetch_leaves_parents(&leaf_ids)
            .await?
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        for leaf_id in leaf_ids {
            let mut tx_cpfp_psbts = Vec::new();
            let mut nodes = Vec::new();

            let Some(mut node) = tree_nodes.get(&leaf_id) else {
                return Err(ServiceError::ValidationError(format!(
                    "Leaf ID {leaf_id} not found in the tree",
                )));
            };
            let Some(refund_tx) = &node.refund_tx else {
                return Err(ServiceError::ValidationError(format!(
                    "Leaf ID {leaf_id} does not have a refund transaction",
                )));
            };

            // Loop through the leaf's ancestors and collect them
            loop {
                nodes.insert(0, node);

                let Some(parent_node_id) = &node.parent_node_id else {
                    break;
                };
                let Some(parent) = tree_nodes.get(parent_node_id) else {
                    return Err(ServiceError::ValidationError(format!(
                        "Parent ID {parent_node_id} not found in the tree",
                    )));
                };
                trace!(
                    "Unilateral exit parent {}, txid {}",
                    parent.id,
                    parent.node_tx.compute_txid()
                );
                node = parent;
            }

            // For each node, check it hasn't already been processed and create a
            // child PSBT for its node tx. If the node is a leaf node, create a
            // child PSBT also for its refund tx.
            for node in nodes {
                let txid = node.node_tx.compute_txid();
                if checked_txs.contains(&txid) {
                    continue;
                }

                checked_txs.insert(txid);

                // Create the PSBT to fee bump the node tx
                let child_psbt = create_tx_cpfp_psbt(&node.node_tx, &mut inputs, fee_rate)?;

                tx_cpfp_psbts.push(TxCpfpPsbt {
                    parent_tx: node.node_tx.clone(),
                    child_psbt,
                });

                if node.id == leaf_id {
                    // Create the PSBT to fee bump the leaf refund tx
                    let child_psbt = create_tx_cpfp_psbt(refund_tx, &mut inputs, fee_rate)?;

                    tx_cpfp_psbts.push(TxCpfpPsbt {
                        parent_tx: refund_tx.clone(),
                        child_psbt,
                    });
                }
            }

            all_leaf_tx_cpfp_psbts.push(LeafTxCpfpPsbts {
                leaf_id,
                tx_cpfp_psbts,
            });
        }

        Ok(all_leaf_tx_cpfp_psbts)
    }

    async fn fetch_leaves_parents(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<TreeNode>, ServiceError> {
        if leaf_ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = &self.operator_pool.get_coordinator().client;
        let nodes = pager(
            |f| self.fetch_leaves_parents_inner(client, leaf_ids, f),
            PagingFilter::default(),
        )
        .await?;

        Ok(nodes.items)
    }

    async fn fetch_leaves_parents_inner(
        &self,
        client: &SparkRpcClient,
        leaf_ids: &[TreeNodeId],
        paging: PagingFilter,
    ) -> Result<PagingResult<TreeNode>, ServiceError> {
        trace!(
            "Fetching leaves parents with limit: {:?}, offset: {:?}",
            paging.limit, paging.offset
        );
        let source = Source::NodeIds(TreeNodeIds {
            node_ids: leaf_ids.iter().map(|id| id.to_string()).collect(),
        });
        let nodes = client
            .query_nodes(QueryNodesRequest {
                include_parents: true,
                limit: paging.limit as i64,
                offset: paging.offset as i64,
                network: self.network.to_proto_network().into(),
                source: Some(source),
                statuses: vec![],
            })
            .await?;
        Ok(PagingResult {
            items: nodes
                .nodes
                .into_values()
                .map(TreeNode::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    ServiceError::Generic(format!("Failed to deserialize leaves: {e:?}"))
                })?,
            next: paging.next_from_offset(nodes.offset),
        })
    }
}

/// Creates a Partially Signed Bitcoin Transaction (PSBT) to CPFP a parent transaction.
///
/// This function creates a PSBT that spends from both CPFP inputs and the ephemeral anchor output
/// of the parent transaction. The resulting PSBT can be signed and broadcast to CPFP the parent
/// transaction with a fee.
///
/// # Arguments
/// * `tx` - The parent transaction to be CPFP'd
/// * `inputs` - A mutable vector of CPFP inputs for fee payment, will be updated with the change output
/// * `fee_rate` - The desired fee rate in satoshis per vbyte
///
/// # Returns
/// A Result containing the PSBT or an error
fn create_tx_cpfp_psbt(
    tx: &Transaction,
    inputs: &mut Vec<CpfpInput>,
    fee_rate: u64,
) -> Result<psbt::Psbt, ServiceError> {
    use bitcoin::psbt::{Input as PsbtInput, Output as PsbtOutput, Psbt};

    // Find the ephemeral anchor output in the parent transaction
    let (vout, anchor_tx_out) = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, tx_out)| is_ephemeral_anchor_output(tx_out))
        .ok_or(ServiceError::ValidationError(
            "Ephemeral anchor output not found".to_string(),
        ))?;

    // We need at least one input for fee payment
    if inputs.is_empty() {
        return Err(ServiceError::ValidationError(
            "At least one CPFP input is required for fee bumping".to_string(),
        ));
    }

    // Calculate total available value from all inputs
    let total_input_value: u64 = inputs.iter().map(|i| i.witness_utxo.value.to_sat()).sum();

    // Change output reuses the first input's scriptPubKey
    let change_script_pubkey = inputs[0].witness_utxo.script_pubkey.clone();
    let first_signed_input_weight = inputs[0].signed_input_weight;

    // Create transaction inputs for all CPFP inputs plus the ephemeral anchor
    let mut tx_inputs = Vec::with_capacity(inputs.len() + 1);

    // Add all CPFP inputs
    // TODO: Improve UTXO selection for fees
    for cpfp_input in inputs.iter() {
        tx_inputs.push(TxIn {
            previous_output: cpfp_input.outpoint,
            ..Default::default()
        });
    }

    // Add the ephemeral anchor input
    tx_inputs.push(TxIn {
        previous_output: OutPoint {
            txid: tx.compute_txid(),
            vout: vout as u32,
        },
        ..Default::default()
    });

    // Calculate the maximum child transaction weight in weight units (WU).
    // Computing in WU avoids rounding errors from per-component vbyte estimates.
    //
    // Non-witness bytes cost 4 WU each, witness bytes cost 1 WU each.
    // Anchor witness: count(1) = 1 byte (empty witness)
    // Overhead non-witness: version(4) + input_count(1) + output_count(1) + locktime(4) = 10 bytes
    // Overhead witness: marker(1) + flag(1) = 2 bytes
    let input_weight: u64 = inputs.iter().map(|i| i.signed_input_weight).sum();
    // Anchor input: 41 * 4 + 1 = 165 WU
    let anchor_weight: u64 = 165;
    // Output weight: (value(8) + scriptPubKey_len(1) + scriptPubKey(N)) * 4
    let output_weight: u64 = (9 + change_script_pubkey.len() as u64) * 4;
    // 10 * 4 + 2 = 42 WU
    let overhead_weight: u64 = 42;
    let child_weight = input_weight + anchor_weight + output_weight + overhead_weight;
    trace!(
        "Estimated child weight: {} WU ({} vbytes)",
        child_weight,
        child_weight.div_ceil(4)
    );

    // For package relay, the fee must cover both parent and child at the target rate.
    // The parent tx has no fee (ephemeral anchor), so the child pays for the whole package.
    // Fee is calculated from weight directly: ceil(fee_rate * weight / 4) to ensure we
    // at least meet the target rate.
    let parent_weight = tx.weight().to_wu();
    let package_weight = parent_weight + child_weight;
    trace!(
        "Parent: {} WU, package total: {} WU ({} vbytes)",
        parent_weight,
        package_weight,
        package_weight.div_ceil(4)
    );

    let fee_amount = (fee_rate * package_weight).div_ceil(4);
    trace!("Calculated fee: {} sats", fee_amount);

    // Adjust output value to account for fees
    let adjusted_output_value = total_input_value.saturating_sub(fee_amount);
    trace!("Remaining value: {} sats", adjusted_output_value);

    // Make sure there's enough value to pay the fee
    if adjusted_output_value == 0 {
        return Err(ServiceError::ValidationError(
            "CPFP input value is too low to cover the fee".to_string(),
        ));
    }

    // Create the base transaction structure
    let fee_bump_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: vec![TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: change_script_pubkey.clone(),
        }],
    };

    // Create a PSBT from the transaction
    let mut psbt = Psbt::from_unsigned_tx(fee_bump_tx.clone())
        .map_err(|e| ServiceError::ValidationError(format!("Failed to create PSBT: {e}")))?;

    // Add PSBT input information for all inputs
    for (i, cpfp_input) in inputs.iter().enumerate() {
        psbt.inputs[i] = PsbtInput {
            witness_utxo: Some(cpfp_input.witness_utxo.clone()),
            ..Default::default()
        };
    }

    // Add information for the last input (the anchor input)
    // Although no signing is needed for the anchor since it uses OP_TRUE,
    // we still provide the witness UTXO information for completeness
    psbt.inputs[inputs.len()] = PsbtInput {
        witness_utxo: Some(anchor_tx_out.clone()),
        ..Default::default()
    };

    // Add details for the output
    psbt.outputs[0] = PsbtOutput::default();

    // Replace all consumed inputs with just the change output
    *inputs = vec![CpfpInput {
        outpoint: OutPoint {
            txid: fee_bump_tx.compute_txid(),
            vout: 0,
        },
        witness_utxo: TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: change_script_pubkey,
        },
        signed_input_weight: first_signed_input_weight,
    }];

    Ok(psbt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        Address, CompressedPublicKey, ScriptBuf,
        hashes::Hash,
        key::Secp256k1,
        secp256k1::{PublicKey, SecretKey, rand},
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// P2WPKH signed input weight: 41 * 4 + 108 = 272 WU
    const P2WPKH_INPUT_WEIGHT: u64 = 272;
    /// P2TR signed input weight: 41 * 4 + 66 = 230 WU
    const P2TR_INPUT_WEIGHT: u64 = 230;

    /// Creates a transaction with an ephemeral anchor output for testing.
    fn create_test_transaction_with_anchor() -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
            }],
        }
    }

    fn p2wpkh_script(pubkey: PublicKey) -> ScriptBuf {
        Address::p2wpkh(&CompressedPublicKey(pubkey), bitcoin::Network::Testnet).script_pubkey()
    }

    fn p2tr_script(pubkey: PublicKey) -> ScriptBuf {
        let secp = Secp256k1::new();
        let (xonly, _) = pubkey.x_only_public_key();
        Address::p2tr(&secp, xonly, None, bitcoin::Network::Testnet).script_pubkey()
    }

    fn create_test_input_p2wpkh(pubkey: PublicKey, value: u64) -> CpfpInput {
        let random_bytes = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();
        CpfpInput {
            outpoint: OutPoint { txid, vout: 0 },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: p2wpkh_script(pubkey),
            },
            signed_input_weight: P2WPKH_INPUT_WEIGHT,
        }
    }

    fn create_test_input_p2tr(pubkey: PublicKey, value: u64) -> CpfpInput {
        let random_bytes = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();
        CpfpInput {
            outpoint: OutPoint { txid, vout: 0 },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey: p2tr_script(pubkey),
            },
            signed_input_weight: P2TR_INPUT_WEIGHT,
        }
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_success() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2wpkh(pubkey, 10_000)];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 2); // One for our input, one for the anchor
        assert_eq!(psbt.outputs.len(), 1); // Change output

        // Verify the output value accounts for fees (package = parent + child in WU)
        let parent_wu = tx.weight().to_wu();
        // P2WPKH scriptPubKey is 22 bytes → output weight = (9 + 22) * 4 = 124
        let child_wu: u64 = 272 + 165 + 124 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = 10_000 - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify inputs array has been updated with the change output
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].witness_utxo.value.to_sat(), expected_output_value);
        assert_eq!(inputs[0].outpoint.vout, 0);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_multiple_inputs() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![
            create_test_input_p2wpkh(pubkey, 5_000),
            create_test_input_p2wpkh(pubkey, 3_000),
            create_test_input_p2wpkh(pubkey, 2_000),
        ];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 4); // Three inputs + anchor
        assert_eq!(psbt.outputs.len(), 1);

        let total_input_value = 5_000 + 3_000 + 2_000;
        let parent_wu = tx.weight().to_wu();
        let child_wu: u64 = (3 * 272) + 165 + 124 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = total_input_value - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].witness_utxo.value.to_sat(), expected_output_value);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_no_inputs() {
        let tx = create_test_transaction_with_anchor();
        let mut inputs = Vec::new();
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, 10);
        assert!(result.is_err());
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_insufficient_value() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2wpkh(pubkey, 10)];
        let fee_rate = 100;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_err());
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_no_anchor_output() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: p2wpkh_script(pubkey),
            }],
        };

        let mut inputs = vec![create_test_input_p2wpkh(pubkey, 10_000)];
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, 10);
        assert!(result.is_err());
        if let Err(ServiceError::ValidationError(msg)) = result {
            assert!(msg.contains("Ephemeral anchor output not found"));
        } else {
            panic!("Expected ValidationError");
        }
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_p2tr_input() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![create_test_input_p2tr(pubkey, 10_000)];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 2);
        assert_eq!(psbt.outputs.len(), 1);

        // P2TR scriptPubKey is 34 bytes → output weight = (9 + 34) * 4 = 172
        let parent_wu = tx.weight().to_wu();
        let child_wu: u64 = 230 + 165 + 172 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = 10_000 - expected_fee;
        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify the output is a P2TR script
        let script = &psbt.unsigned_tx.output[0].script_pubkey;
        assert!(script.is_p2tr());

        // Verify the change preserves P2TR scriptPubKey and weight
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].witness_utxo.script_pubkey.is_p2tr());
        assert_eq!(inputs[0].signed_input_weight, P2TR_INPUT_WEIGHT);
        assert_eq!(inputs[0].witness_utxo.value.to_sat(), expected_output_value);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_mixed_input_types() {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        let tx = create_test_transaction_with_anchor();
        let mut inputs = vec![
            create_test_input_p2wpkh(pubkey, 5_000),
            create_test_input_p2tr(pubkey, 3_000),
        ];

        let fee_rate = 10;
        let result = create_tx_cpfp_psbt(&tx, &mut inputs, fee_rate);
        assert!(result.is_ok());

        let psbt = result.unwrap();
        assert_eq!(psbt.inputs.len(), 3); // 2 inputs + anchor

        // Mixed: 272 (p2wpkh) + 230 (p2tr) + 165 (anchor) + 124 (p2wpkh output) + 42 (overhead)
        let parent_wu = tx.weight().to_wu();
        let child_wu: u64 = 272 + 230 + 165 + 124 + 42;
        let package_weight = parent_wu + child_wu;
        let expected_fee = (fee_rate * package_weight).div_ceil(4);
        let expected_output_value = 8_000 - expected_fee;
        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Change output uses the first input's scriptPubKey (P2WPKH)
        let script = &psbt.unsigned_tx.output[0].script_pubkey;
        assert!(script.is_p2wpkh());
        // Change carries forward first input's weight
        assert_eq!(inputs[0].signed_input_weight, P2WPKH_INPUT_WEIGHT);
    }

    #[test_all]
    fn test_is_ephemeral_anchor_output() {
        let valid_anchor = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(is_ephemeral_anchor_output(&valid_anchor));

        let non_zero_value = TxOut {
            value: Amount::from_sat(1),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(!is_ephemeral_anchor_output(&non_zero_value));

        let different_script = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51]),
        };
        assert!(!is_ephemeral_anchor_output(&different_script));
    }
}
