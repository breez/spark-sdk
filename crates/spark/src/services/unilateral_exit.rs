use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, Psbt, Transaction, TxIn, TxOut, Txid,
    absolute::LockTime, psbt, secp256k1::PublicKey, transaction::Version,
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

pub struct CpfpUtxo {
    pub txid: Txid,
    pub vout: u32,
    pub value: u64,
    pub pubkey: PublicKey,
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
        mut utxos: Vec<CpfpUtxo>,
    ) -> Result<Vec<LeafTxCpfpPsbts>, ServiceError> {
        if leaf_ids.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one leaf ID is required".to_string(),
            ));
        }
        if utxos.is_empty() {
            return Err(ServiceError::ValidationError(
                "At least one UTXO is required".to_string(),
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
                let child_psbt =
                    create_tx_cpfp_psbt(&node.node_tx, &mut utxos, fee_rate, self.network.into())?;

                tx_cpfp_psbts.push(TxCpfpPsbt {
                    parent_tx: node.node_tx.clone(),
                    child_psbt,
                });

                if node.id == leaf_id {
                    // Create the PSBT to fee bump the leaf refund tx
                    let child_psbt =
                        create_tx_cpfp_psbt(refund_tx, &mut utxos, fee_rate, self.network.into())?;

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
/// This function creates a PSBT that spends from both input UTXOs and the ephemeral anchor output
/// of the parent transaction. The resulting PSBT can be signed and broadcast to CPFP the parent
/// transaction with a fee.
///
/// # Arguments
/// * `tx` - The parent transaction to be CPFP'd
/// * `utxos` - A mutable vector of UTXOs that can be used to pay fees, will be updated with the change UTXO
/// * `fee_rate` - The desired fee rate in satoshis per vbyte
/// * `network` - The Bitcoin network (mainnet, testnet, etc.)
///
/// # Returns
/// A Result containing the PSBT or an error
fn create_tx_cpfp_psbt(
    tx: &Transaction,
    utxos: &mut Vec<CpfpUtxo>,
    fee_rate: u64,
    network: bitcoin::Network,
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

    // We need at least one UTXO for fee payment
    if utxos.is_empty() {
        return Err(ServiceError::ValidationError(
            "At least one UTXO is required for fee bumping".to_string(),
        ));
    }

    // Calculate total available value from all UTXOs
    let total_utxo_value: u64 = utxos.iter().map(|utxo| utxo.value).sum();

    // Use the first UTXO's pubkey for the output
    let first_pubkey = utxos[0].pubkey;
    let output_script_pubkey = Address::p2wpkh(&CompressedPublicKey(first_pubkey), network).into();

    // Create inputs for all UTXOs plus the ephemeral anchor
    let mut inputs = Vec::with_capacity(utxos.len() + 1);

    // Add all UTXO inputs
    // TODO: Improve UTXO selection for fees
    for utxo in utxos.iter() {
        inputs.push(TxIn {
            previous_output: OutPoint {
                txid: utxo.txid,
                vout: utxo.vout,
            },
            ..Default::default()
        });
    }

    // Add the ephemeral anchor input
    inputs.push(TxIn {
        previous_output: OutPoint {
            txid: tx.compute_txid(),
            vout: vout as u32,
        },
        ..Default::default()
    });

    // Calculate the approximate transaction size in vbytes
    // P2WPKH inputs: ~68 vbytes each (outpoint + script + witnesses)
    // Anchor input: ~41 vbytes (smaller because no signature needed for ephemeral anchor)
    // P2WPKH output: ~31 vbytes
    // Transaction overhead: ~10 vbytes
    // TODO: calculate a better estimate of tx size. p2tr inputs have a different size.
    //       For most input types we can calculate exact sizes.
    let tx_size_vbytes = (utxos.len() as u64 * 68) + 41 + 31 + 10;
    trace!("Estimated transaction size: {} vbytes", tx_size_vbytes);

    // Calculate fee based on fee rate (fee_rate is in sat/vbyte)
    let fee_amount = fee_rate * tx_size_vbytes;
    trace!("Calculated fee: {} sats", fee_amount);

    // Adjust output value to account for fees
    let adjusted_output_value = total_utxo_value.saturating_sub(fee_amount);
    trace!("Remaining UTXO value: {} sats", adjusted_output_value);

    // Make sure there's enough value to pay the fee
    if adjusted_output_value == 0 {
        return Err(ServiceError::ValidationError(
            "UTXOs value is too low to cover the fee".to_string(),
        ));
    }

    // Create the base transaction structure
    let fee_bump_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: inputs,
        output: vec![TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: output_script_pubkey,
        }],
    };

    // Create a PSBT from the transaction
    let mut psbt = Psbt::from_unsigned_tx(fee_bump_tx.clone())
        .map_err(|e| ServiceError::ValidationError(format!("Failed to create PSBT: {e}")))?;

    // Add PSBT input information for all inputs
    for (i, utxo) in utxos.iter().enumerate() {
        // Add witness UTXO information required for signing
        // This provides information about the output being spent
        let input = PsbtInput {
            witness_utxo: Some(TxOut {
                value: Amount::from_sat(utxo.value),
                // TODO: Support p2tr inputs
                script_pubkey: Address::p2wpkh(&CompressedPublicKey(utxo.pubkey), network)
                    .script_pubkey(),
            }),
            ..Default::default()
        };

        psbt.inputs[i] = input;
    }

    // Add information for the last input (the anchor input)
    // Although no signing is needed for the anchor since it uses OP_TRUE,
    // we still provide the witness UTXO information for completeness
    let anchor_input = PsbtInput {
        witness_utxo: Some(anchor_tx_out.clone()),
        ..Default::default()
    };
    psbt.inputs[utxos.len()] = anchor_input;

    // Add details for the output
    psbt.outputs[0] = PsbtOutput::default();

    // Replace all consumed UTXOs with just the change output
    *utxos = vec![CpfpUtxo {
        txid: fee_bump_tx.compute_txid(),
        vout: 0,
        value: adjusted_output_value,
        pubkey: first_pubkey,
    }];

    Ok(psbt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        ScriptBuf,
        hashes::Hash,
        key::Secp256k1,
        secp256k1::{SecretKey, rand},
    };
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// Creates a transaction with an ephemeral anchor output for testing.
    fn create_test_transaction_with_anchor() -> Transaction {
        // Create a simple transaction with an ephemeral anchor output
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

    /// Creates a test UTXO with a random txid and the given pubkey.
    fn create_test_utxo(pubkey: PublicKey, value: u64) -> CpfpUtxo {
        // Create a random txid
        let random_bytes = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();

        CpfpUtxo {
            txid,
            vout: 0,
            value,
            pubkey,
        }
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_success() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Create a test UTXO with sufficient value
        let mut utxos = vec![create_test_utxo(pubkey, 10_000)];

        // Set a reasonable fee rate (10 sats/vbyte)
        let fee_rate = 10;

        // Call the function
        let result = create_tx_cpfp_psbt(&tx, &mut utxos, fee_rate, bitcoin::Network::Testnet);

        // Verify the result
        assert!(result.is_ok());

        let psbt = result.unwrap();

        // Validate the PSBT
        assert_eq!(psbt.inputs.len(), 2); // One for our UTXO, one for the anchor
        assert_eq!(psbt.outputs.len(), 1); // Change output

        // Verify the output value accounts for fees
        let estimated_size = 68 + 41 + 31 + 10; // UTXO input + anchor input + output + overhead
        let expected_fee = fee_rate * estimated_size;
        let expected_output_value = 10_000 - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify our UTXOs array has been updated with the change output
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].value, expected_output_value);
        assert_eq!(utxos[0].vout, 0);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_multiple_utxos() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Create multiple test UTXOs
        let mut utxos = vec![
            create_test_utxo(pubkey, 5_000),
            create_test_utxo(pubkey, 3_000),
            create_test_utxo(pubkey, 2_000),
        ];

        // Set a reasonable fee rate
        let fee_rate = 10;

        // Call the function
        let result = create_tx_cpfp_psbt(&tx, &mut utxos, fee_rate, bitcoin::Network::Testnet);

        // Verify the result
        assert!(result.is_ok());

        let psbt = result.unwrap();

        // Validate the PSBT
        assert_eq!(psbt.inputs.len(), 4); // Three UTXOs + anchor
        assert_eq!(psbt.outputs.len(), 1); // Change output

        // Verify the total input value (excluding anchor which is 0)
        let total_input_value = 5_000 + 3_000 + 2_000;

        // Verify the output value accounts for fees
        let estimated_size = (3 * 68) + 41 + 31 + 10; // 3 UTXO inputs + anchor input + output + overhead
        let expected_fee = fee_rate * estimated_size;
        let expected_output_value = total_input_value - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify our UTXOs array has been updated with the change output
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].value, expected_output_value);
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_no_utxos() {
        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Empty UTXOs vector
        let mut utxos = Vec::new();

        // Call the function
        let result = create_tx_cpfp_psbt(&tx, &mut utxos, 10, bitcoin::Network::Testnet);

        // Verify the PSBT creation fails
        assert!(result.is_err());
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_insufficient_value() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Create a test UTXO with very low value
        let mut utxos = vec![create_test_utxo(pubkey, 10)];

        // Set a high fee rate to ensure the fee exceeds the UTXO value
        let fee_rate = 100;

        // Call the function
        let result = create_tx_cpfp_psbt(&tx, &mut utxos, fee_rate, bitcoin::Network::Testnet);

        // Verify the PSBT creation fails
        assert!(result.is_err());
    }

    #[test_all]
    fn test_create_tx_cpfp_psbt_no_anchor_output() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction WITHOUT an anchor output (just a regular output)
        let tx = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: Address::p2wpkh(
                    &CompressedPublicKey(pubkey),
                    bitcoin::Network::Testnet,
                )
                .script_pubkey(),
            }],
        };

        let mut utxos = vec![create_test_utxo(pubkey, 10_000)];

        // Call the function
        let result = create_tx_cpfp_psbt(&tx, &mut utxos, 10, bitcoin::Network::Testnet);

        // Should fail because no anchor output was found
        assert!(result.is_err());
        if let Err(ServiceError::ValidationError(msg)) = result {
            assert!(msg.contains("Ephemeral anchor output not found"));
        } else {
            panic!("Expected ValidationError");
        }
    }

    #[test_all]
    fn test_is_ephemeral_anchor_output() {
        // Test case 1: Valid ephemeral anchor output
        let valid_anchor = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(is_ephemeral_anchor_output(&valid_anchor));

        // Test case 2: Non-zero value
        let non_zero_value = TxOut {
            value: Amount::from_sat(1),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(!is_ephemeral_anchor_output(&non_zero_value));

        // Test case 3: Different script
        let different_script = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51]),
        };
        assert!(!is_ephemeral_anchor_output(&different_script));
    }
}
