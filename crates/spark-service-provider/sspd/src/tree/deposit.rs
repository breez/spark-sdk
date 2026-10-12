use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::{
    Amount, OutPoint, TapSighash, Transaction, TxOut, Txid,
    consensus::{deserialize, serialize},
    secp256k1::PublicKey,
};

use crate::frost::{SignedShare, operator_commitments, sign_share};
use spark::{
    Network,
    bitcoin::sighash_from_tx,
    core::initial_zero_timelock_sequence,
    operator::{
        OperatorPool,
        rpc::{self as operator_rpc, spark::UserSignedTxSigningJob},
    },
    signer::{SecretSource, Signer, signing_path},
    tree::{TreeNode, TreeNodeId, TreeNodeStatus},
    utils::transactions::create_initial_timelock_refund_txs,
};

use super::builder::{TreeBlueprint, create_branch_spark_tx, p2tr_script_pubkey};

use spark::services::ServiceError;

/// The operators refuse to finalize more nodes than this in one call.
pub const DEFAULT_MAX_NODES_PER_REQUEST: usize = 1000;

fn fits_in_one_call(blueprint: &TreeBlueprint, max_nodes: usize) -> bool {
    nodes_needed(blueprint) <= max_nodes
}

fn split_depth(blueprint: &TreeBlueprint, max_nodes: usize) -> usize {
    let mut deepest_fitting_stub = 1;
    for depth in 1..=MAX_SPLIT_DEPTH {
        let stub_fits = fits_in_one_call(&truncate_at_depth(blueprint, depth), max_nodes);
        if stub_fits {
            deepest_fitting_stub = depth;
        }
        let frontier = frontier_at_depth(blueprint, depth);
        if stub_fits
            && frontier
                .iter()
                .all(|subtree| fits_in_one_call(subtree, max_nodes))
        {
            return depth;
        }
        // Once the frontier is all leaves, a deeper cut changes nothing.
        if frontier.iter().all(|subtree| subtree.is_leaf()) {
            break;
        }
    }
    deepest_fitting_stub
}

fn nodes_needed(blueprint: &TreeBlueprint) -> usize {
    match blueprint {
        TreeBlueprint::Leaf { .. } | TreeBlueprint::Split { .. } => 1,
        TreeBlueprint::Branch { children, .. } => children
            .iter()
            .map(nodes_needed)
            .sum::<usize>()
            .saturating_add(1),
    }
}

fn frontier_at_depth(blueprint: &TreeBlueprint, depth: usize) -> Vec<&TreeBlueprint> {
    if depth == 0 || blueprint.is_leaf() {
        return vec![blueprint];
    }
    blueprint
        .children()
        .iter()
        .flat_map(|child| frontier_at_depth(child, depth.saturating_sub(1)))
        .collect()
}

/// The key subtrees matching [`frontier_at_depth`], walked in the same order.
fn key_frontier_at_depth(key_tree: &KeyTree, depth: usize) -> Vec<&KeyTree> {
    if depth == 0 || key_tree.children.is_empty() {
        return vec![key_tree];
    }
    key_tree
        .children
        .iter()
        .flat_map(|child| key_frontier_at_depth(child, depth.saturating_sub(1)))
        .collect()
}

fn truncate_at_depth(blueprint: &TreeBlueprint, depth: usize) -> TreeBlueprint {
    match blueprint {
        TreeBlueprint::Branch { value, children } if depth > 0 => TreeBlueprint::Branch {
            value: *value,
            children: children
                .iter()
                .map(|child| truncate_at_depth(child, depth.saturating_sub(1)))
                .collect(),
        },
        TreeBlueprint::Branch { value, .. } => TreeBlueprint::Split { value: *value },
        other => other.clone(),
    }
}

/// A balanced binary tree with fewer than 2^64 leaves is at most 64 levels deep.
const MAX_SPLIT_DEPTH: usize = 64;

enum TreeSource {
    OnChainUtxo(operator_rpc::spark::Utxo),
    ParentNodeOutput(operator_rpc::spark::NodeOutput),
}

impl TreeSource {
    fn to_prepare_source(&self) -> operator_rpc::spark::prepare_tree_address_request::Source {
        use operator_rpc::spark::prepare_tree_address_request::Source;
        match self {
            Self::OnChainUtxo(utxo) => Source::OnChainUtxo(utxo.clone()),
            Self::ParentNodeOutput(output) => Source::ParentNodeOutput(output.clone()),
        }
    }

    fn to_create_source(&self) -> crate::operator_rpc::create_tree_request::Source {
        use crate::operator_rpc::create_tree_request::Source;
        match self {
            Self::OnChainUtxo(utxo) => Source::OnChainUtxo(utxo.clone()),
            Self::ParentNodeOutput(output) => Source::ParentNodeOutput(output.clone()),
        }
    }
}

struct NodeBuildData {
    signing_key: SecretSource,
    signing_public_key: PublicKey,
    verifying_key: PublicKey,
    cpfp_tx: Transaction,
    cpfp_refund_tx: Option<Transaction>,
    direct_from_cpfp_refund_tx: Option<Transaction>,
    parent_output: TxOut,
    children: Vec<NodeBuildData>,
}

/// A created tree, with each leaf paired with the id its signing key derives from.
pub struct CreatedTree {
    pub pairs: Vec<(TreeNode, TreeNodeId)>,
    pub nodes: CreatedNodes,
}

/// A subtree's leaves, in tree order, and every node above them.
#[derive(Debug, Default)]
pub struct CreatedNodes {
    pub leaves: Vec<TreeNode>,
    pub branches: Vec<TreeNode>,
}

impl CreatedNodes {
    fn absorb(&mut self, other: CreatedNodes) {
        self.leaves.extend(other.leaves);
        self.branches.extend(other.branches);
    }
}

pub struct TreeDepositService {
    identity_public_key: PublicKey,
    network: Network,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    max_nodes_per_request: usize,
}

impl TreeDepositService {
    pub fn new(
        identity_public_key: PublicKey,
        network: impl Into<Network>,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
    ) -> Self {
        Self {
            identity_public_key,
            network: network.into(),
            operator_pool,
            signer,
            max_nodes_per_request: DEFAULT_MAX_NODES_PER_REQUEST,
        }
    }

    #[must_use]
    pub fn with_max_nodes_per_request(mut self, max: usize) -> Self {
        self.max_nodes_per_request = max.max(1);
        self
    }

    pub async fn plan_deposit_tree(
        &self,
        blueprint: &TreeBlueprint,
    ) -> Result<DepositTreePlan, ServiceError> {
        let mut leaf_ids = Vec::with_capacity(blueprint.leaf_count());
        for _ in 0..blueprint.leaf_count() {
            leaf_ids.push(TreeNodeId::generate());
        }

        let key_tree = self.build_key_tree_bottom_up(blueprint, &leaf_ids).await?;

        Ok(DepositTreePlan {
            root_public_key: key_tree.public_key,
            leaf_ids,
        })
    }

    /// `leaf_ids` must be in tree order (depth-first), matching the blueprint's leaves.
    pub async fn execute_deposit_tree(
        &self,
        blueprint: &TreeBlueprint,
        leaf_ids: &[TreeNodeId],
        verifying_public_key: &PublicKey,
        deposit_tx: Transaction,
        vout: u32,
    ) -> Result<CreatedTree, ServiceError> {
        let deposit_output = deposit_tx
            .output
            .get(vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?;

        let deposit_amount = deposit_output.value.to_sat();
        let leaf_sum: u64 = blueprint.leaf_values().iter().sum();
        if leaf_sum != deposit_amount {
            return Err(ServiceError::Generic(format!(
                "leaf values sum ({leaf_sum}) does not match deposit amount ({deposit_amount})"
            )));
        }

        if leaf_ids.len() != blueprint.leaf_count() {
            return Err(ServiceError::Generic(format!(
                "leaf_ids count ({}) does not match blueprint leaf count ({})",
                leaf_ids.len(),
                blueprint.leaf_count()
            )));
        }

        let key_tree = self.build_key_tree_bottom_up(blueprint, leaf_ids).await?;

        // The operators get the transaction without witnesses: they read only its txid and
        // outputs, and could broadcast a signed copy before the trees are finalized and stored.
        let deposit_txid = deposit_tx.compute_txid();
        let mut unsigned = deposit_tx.clone();
        for input in &mut unsigned.input {
            input.witness.clear();
        }
        let utxo = operator_rpc::spark::Utxo {
            raw_tx: serialize(&unsigned),
            vout,
            network: self.network.to_proto_network() as i32,
            txid: hex::decode(deposit_txid.to_string())
                .map_err(|_| ServiceError::InvalidTransaction)?,
        };

        let nodes = self
            .create_subtree(
                TreeSource::OnChainUtxo(utxo),
                blueprint,
                &key_tree,
                &deposit_tx,
                vout,
                Some(verifying_public_key),
            )
            .await?;

        if nodes.leaves.len() != leaf_ids.len() {
            return Err(ServiceError::Generic(format!(
                "leaf count mismatch: {} nodes vs {} leaf_ids",
                nodes.leaves.len(),
                leaf_ids.len()
            )));
        }

        // `nodes.leaves` and `leaf_ids` are both in tree order.
        Ok(CreatedTree {
            pairs: nodes
                .leaves
                .iter()
                .cloned()
                .zip(leaf_ids.iter().cloned())
                .collect(),
            nodes,
        })
    }

    /// A tree over the per-call node limit is created in layers: the top first,
    /// ending in [`TreeBlueprint::Split`] nodes, then the subtree under each split.
    #[allow(clippy::too_many_arguments)]
    async fn create_subtree(
        &self,
        source: TreeSource,
        blueprint: &TreeBlueprint,
        key_tree: &KeyTree,
        parent_tx: &Transaction,
        parent_vout: u32,
        expected_verifying_key: Option<&PublicKey>,
    ) -> Result<CreatedNodes, ServiceError> {
        if fits_in_one_call(blueprint, self.max_nodes_per_request) {
            let call = self
                .create_subtree_in_one_call(
                    source,
                    blueprint,
                    key_tree,
                    parent_tx,
                    parent_vout,
                    expected_verifying_key,
                )
                .await?;
            return Ok(CreatedNodes {
                leaves: call.frontier,
                branches: call.branches,
            });
        }

        let depth = split_depth(blueprint, self.max_nodes_per_request);
        let top = truncate_at_depth(blueprint, depth);
        let subtrees = frontier_at_depth(blueprint, depth);
        let sub_keys = key_frontier_at_depth(key_tree, depth);

        let top_nodes = self
            .create_subtree_in_one_call(
                source,
                &top,
                key_tree,
                parent_tx,
                parent_vout,
                expected_verifying_key,
            )
            .await?;
        if top_nodes.frontier.len() != subtrees.len() {
            return Err(ServiceError::Generic(format!(
                "layer returned {} nodes for {} subtrees",
                top_nodes.frontier.len(),
                subtrees.len()
            )));
        }

        let mut created = CreatedNodes {
            leaves: Vec::new(),
            branches: top_nodes.branches,
        };

        // Each subtree spends a different split's output, so they are created concurrently.
        let extensions = subtrees
            .into_iter()
            .zip(sub_keys)
            .zip(top_nodes.frontier)
            .map(|((child_bp, child_kt), node)| async move {
                if child_bp.is_leaf() {
                    return Ok::<_, ServiceError>(CreatedNodes {
                        leaves: vec![node],
                        branches: Vec::new(),
                    });
                }
                let parent_output = operator_rpc::spark::NodeOutput {
                    node_id: node.id.to_string(),
                    vout: 0,
                };
                let mut below = Box::pin(self.create_subtree(
                    TreeSource::ParentNodeOutput(parent_output),
                    child_bp,
                    child_kt,
                    &node.node_tx,
                    0,
                    None,
                ))
                .await?;
                below.branches.push(node);
                Ok(below)
            });

        for part in futures::future::try_join_all(extensions).await? {
            created.absorb(part);
        }
        Ok(created)
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_subtree_in_one_call(
        &self,
        source: TreeSource,
        blueprint: &TreeBlueprint,
        key_tree: &KeyTree,
        parent_tx: &Transaction,
        parent_vout: u32,
        expected_verifying_key: Option<&PublicKey>,
    ) -> Result<CallNodes, ServiceError> {
        let address_request_node = Self::build_address_request_node(blueprint, key_tree);

        let address_response = crate::operator_rpc::prepare_tree_address(
            &self.operator_pool.get_coordinator().client,
            operator_rpc::spark::PrepareTreeAddressRequest {
                source: Some(source.to_prepare_source()),
                node: Some(address_request_node),
                user_identity_public_key: self.identity_public_key.serialize().to_vec(),
            },
        )
        .await?;

        let address_node = address_response.node.ok_or(ServiceError::Generic(
            "Missing address node in response".to_string(),
        ))?;

        let verifying_key_tree = Self::extract_verifying_key_tree(&address_node)?;
        if let Some(expected) = expected_verifying_key
            && verifying_key_tree.key != *expected
        {
            return Err(ServiceError::InvalidVerifyingKey);
        }

        let node_tree = self
            .build_node_tree(
                blueprint,
                key_tree,
                &verifying_key_tree,
                parent_tx,
                parent_vout,
            )
            .await?;

        let creation_node = self.sign_node_tree(&node_tree).await?;

        let created = crate::operator_rpc::create_tree(
            &self.operator_pool.get_coordinator().client,
            crate::operator_rpc::CreateTreeRequest {
                source: Some(source.to_create_source()),
                node: Some(creation_node),
                user_identity_public_key: self.identity_public_key.serialize().to_vec(),
            },
        )
        .await?;

        // Matched to the plan by transaction id, so the nodes come out in tree
        // order whatever order the operators list them in.
        let finalized = created
            .nodes
            .into_iter()
            .map(|node| {
                let node = Self::proto_node_to_tree_node(node)?;
                Ok((node.node_tx.compute_txid(), node))
            })
            .collect::<Result<HashMap<Txid, TreeNode>, ServiceError>>()?;
        let mut call = CallNodes::default();
        collect_call_nodes(&node_tree, &finalized, &mut call)?;
        Ok(call)
    }

    /// A branch's key is the sum of its children's, as the operators require.
    async fn build_key_tree_bottom_up(
        &self,
        blueprint: &TreeBlueprint,
        leaf_ids: &[TreeNodeId],
    ) -> Result<KeyTree, ServiceError> {
        if leaf_ids.len() != blueprint.leaf_count() {
            return Err(ServiceError::Generic(format!(
                "leaf_ids count ({}) does not match blueprint leaf count ({})",
                leaf_ids.len(),
                blueprint.leaf_count()
            )));
        }

        match blueprint {
            TreeBlueprint::Split { .. } => Err(ServiceError::Generic(
                "a split node has no keys of its own".to_string(),
            )),
            TreeBlueprint::Leaf { .. } => {
                let leaf_id = leaf_ids.first().ok_or_else(|| {
                    ServiceError::Generic("a leaf has no id to derive its key from".to_string())
                })?;
                let signing_key = SecretSource::Derived(signing_path(leaf_id)?);
                let public_key = self.signer.public_key_from_secret(&signing_key).await?;
                Ok(KeyTree {
                    signing_key,
                    public_key,
                    children: vec![],
                })
            }
            TreeBlueprint::Branch { children, .. } => {
                let mut offset: usize = 0;
                let mut slices = Vec::with_capacity(children.len());
                for child in children {
                    let count = child.leaf_count();
                    let end = offset.saturating_add(count);
                    let ids = leaf_ids.get(offset..end).ok_or_else(|| {
                        ServiceError::Generic("fewer leaf ids than the tree has leaves".to_string())
                    })?;
                    slices.push((child, ids));
                    offset = end;
                }

                let child_trees = futures::future::try_join_all(
                    slices
                        .into_iter()
                        .map(|(child, ids)| Box::pin(self.build_key_tree_bottom_up(child, ids))),
                )
                .await?;

                let child_keys: Vec<SecretSource> = child_trees
                    .iter()
                    .map(|ct| ct.signing_key.clone())
                    .collect();
                let (signing_key, public_key) = self
                    .signer
                    .combine_signing_keys(&child_keys)
                    .await
                    .map_err(|e| ServiceError::Generic(format!("key combination failed: {e}")))?;

                Ok(KeyTree {
                    signing_key,
                    public_key,
                    children: child_trees,
                })
            }
        }
    }

    fn build_address_request_node(
        blueprint: &TreeBlueprint,
        key_tree: &KeyTree,
    ) -> operator_rpc::spark::AddressRequestNode {
        let children = match blueprint {
            TreeBlueprint::Leaf { .. } | TreeBlueprint::Split { .. } => vec![],
            TreeBlueprint::Branch {
                children: bp_children,
                ..
            } => bp_children
                .iter()
                .zip(key_tree.children.iter())
                .map(|(bp, kt)| Self::build_address_request_node(bp, kt))
                .collect(),
        };

        operator_rpc::spark::AddressRequestNode {
            user_public_key: key_tree.public_key.serialize().to_vec(),
            children,
        }
    }

    fn extract_verifying_key_tree(
        node: &operator_rpc::spark::AddressNode,
    ) -> Result<VerifyingKeyTree, ServiceError> {
        let address = node.address.as_ref().ok_or(ServiceError::Generic(
            "Missing address in AddressNode".to_string(),
        ))?;
        let key = PublicKey::from_slice(&address.verifying_key)
            .map_err(|_| ServiceError::InvalidVerifyingKey)?;

        let children = node
            .children
            .iter()
            .map(Self::extract_verifying_key_tree)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(VerifyingKeyTree { key, children })
    }

    async fn build_node_tree(
        &self,
        blueprint: &TreeBlueprint,
        key_tree: &KeyTree,
        vk_tree: &VerifyingKeyTree,
        parent_tx: &Transaction,
        parent_vout: u32,
    ) -> Result<NodeBuildData, ServiceError> {
        let parent_output = parent_tx
            .output
            .get(parent_vout as usize)
            .ok_or(ServiceError::InvalidOutputIndex)?
            .clone();

        let verifying_key = vk_tree.key;
        let is_branch = matches!(blueprint, TreeBlueprint::Branch { .. });

        let (cpfp_sequence, _direct_sequence) = initial_zero_timelock_sequence();

        let parent_outpoint = OutPoint {
            txid: parent_tx.compute_txid(),
            vout: parent_vout,
        };

        let cpfp_tx = if is_branch {
            let bp_children = blueprint.children();
            let child_outputs: Vec<_> = bp_children
                .iter()
                .zip(vk_tree.children.iter())
                .map(|(child_bp, child_vk)| {
                    (
                        Amount::from_sat(child_bp.value()),
                        p2tr_script_pubkey(&child_vk.key, self.network),
                    )
                })
                .collect();
            create_branch_spark_tx(parent_outpoint, cpfp_sequence, &child_outputs, true)
        } else {
            spark::utils::transactions::create_spark_tx(
                parent_outpoint,
                cpfp_sequence,
                parent_output.value,
                p2tr_script_pubkey(&verifying_key, self.network),
                false,
                true,
            )
        };

        // Only a leaf is refunded: a split's refund would spend the output its
        // subtree's root spends.
        let (cpfp_refund_tx, direct_from_cpfp_refund_tx) = if blueprint.is_leaf() {
            let refund_txs = create_initial_timelock_refund_txs(
                &cpfp_tx,
                None,
                &key_tree.public_key,
                self.network,
            )?;
            let dfcr_tx = refund_txs.direct_from_cpfp_tx.ok_or(ServiceError::Generic(
                "Missing direct_from_cpfp refund tx".to_string(),
            ))?;
            (Some(refund_txs.cpfp_tx), Some(dfcr_tx))
        } else {
            (None, None)
        };

        let children = if is_branch {
            let cpfp_tx = &cpfp_tx;
            futures::future::try_join_all(
                blueprint
                    .children()
                    .iter()
                    .zip(key_tree.children.iter())
                    .zip(vk_tree.children.iter())
                    .enumerate()
                    .map(|(i, ((child_bp, child_kt), child_vk))| {
                        #[allow(clippy::cast_possible_truncation)]
                        let vout = i as u32;
                        Box::pin(self.build_node_tree(child_bp, child_kt, child_vk, cpfp_tx, vout))
                    }),
            )
            .await?
        } else {
            vec![]
        };

        Ok(NodeBuildData {
            signing_key: key_tree.signing_key.clone(),
            signing_public_key: key_tree.public_key,
            verifying_key,
            cpfp_tx,
            cpfp_refund_tx,
            direct_from_cpfp_refund_tx,
            parent_output,
            children,
        })
    }

    /// Signs every transaction of the tree the operators co-sign, against
    /// commitments the operators make for this call.
    async fn sign_node_tree(
        &self,
        root: &NodeBuildData,
    ) -> Result<crate::operator_rpc::CreationNode, ServiceError> {
        let mut unsigned = Vec::new();
        collect_unsigned(root, &mut unsigned)?;

        let pairs = u32::try_from(unsigned.len().div_ceil(2))
            .map_err(|_| ServiceError::Generic("the tree has too many transactions".to_string()))?;
        let commitments =
            operator_commitments(&self.operator_pool.get_coordinator().client, pairs, 2).await?;
        if commitments.len() < unsigned.len() {
            return Err(ServiceError::Generic(format!(
                "asked the operators for {} signing commitments, got {}",
                unsigned.len(),
                commitments.len()
            )));
        }

        let mut shares = Vec::with_capacity(unsigned.len());
        for ((sighash, data), commitments) in unsigned.into_iter().zip(commitments) {
            shares.push(
                sign_share(
                    &self.signer,
                    &sighash,
                    &data.signing_key,
                    &data.signing_public_key,
                    &data.verifying_key,
                    commitments,
                )
                .await?,
            );
        }
        to_creation_node(root, &mut shares.into_iter())
    }

    fn proto_node_to_tree_node(
        node: operator_rpc::spark::TreeNode,
    ) -> Result<TreeNode, ServiceError> {
        let signing_keyshare = node.signing_keyshare.ok_or(ServiceError::Generic(
            "missing signing keyshare".to_string(),
        ))?;

        Ok(TreeNode {
            id: node
                .id
                .parse()
                .map_err(|_| ServiceError::InvalidNodeId(node.id))?,
            tree_id: node.tree_id,
            value: node.value,
            parent_node_id: match node.parent_node_id {
                Some(id) => Some(id.parse().map_err(|_| ServiceError::InvalidNodeId(id))?),
                None => None,
            },
            node_tx: deserialize(&node.node_tx).map_err(|_| ServiceError::InvalidTransaction)?,
            refund_tx: if node.refund_tx.is_empty() {
                None
            } else {
                Some(deserialize(&node.refund_tx).map_err(|_| ServiceError::InvalidTransaction)?)
            },
            direct_tx: if node.direct_tx.is_empty() {
                None
            } else {
                Some(deserialize(&node.direct_tx).map_err(|_| ServiceError::InvalidTransaction)?)
            },
            direct_refund_tx: if node.direct_refund_tx.is_empty() {
                None
            } else {
                Some(
                    deserialize(&node.direct_refund_tx)
                        .map_err(|_| ServiceError::InvalidTransaction)?,
                )
            },
            direct_from_cpfp_refund_tx: if node.direct_from_cpfp_refund_tx.is_empty() {
                None
            } else {
                Some(
                    deserialize(&node.direct_from_cpfp_refund_tx)
                        .map_err(|_| ServiceError::InvalidTransaction)?,
                )
            },
            vout: node.vout,
            verifying_public_key: PublicKey::from_slice(&node.verifying_public_key)
                .map_err(|_| ServiceError::InvalidVerifyingKey)?,
            owner_identity_public_key: if node.owner_identity_public_key.is_empty() {
                None
            } else {
                Some(
                    PublicKey::from_slice(&node.owner_identity_public_key)
                        .map_err(|_| ServiceError::InvalidPublicKey)?,
                )
            },
            signing_keyshare: signing_keyshare.try_into()?,
            status: TreeNodeStatus::from(node.status.as_str()),
        })
    }
}

pub struct DepositTreePlan {
    /// The root's signing key: the sum of every leaf's.
    pub root_public_key: PublicKey,
    /// The ids the leaves' signing keys derive from, in tree order.
    pub leaf_ids: Vec<TreeNodeId>,
}

struct KeyTree {
    signing_key: SecretSource,
    public_key: PublicKey,
    children: Vec<KeyTree>,
}

struct VerifyingKeyTree {
    key: PublicKey,
    children: Vec<VerifyingKeyTree>,
}

/// One creation call's leaves and splits in tree order, and the nodes above them.
#[derive(Default)]
struct CallNodes {
    frontier: Vec<TreeNode>,
    branches: Vec<TreeNode>,
}

/// The sighash of every transaction the operators co-sign, in the order
/// [`to_creation_node`] hands out the shares.
fn collect_unsigned<'a>(
    data: &'a NodeBuildData,
    out: &mut Vec<(TapSighash, &'a NodeBuildData)>,
) -> Result<(), ServiceError> {
    out.push((
        sighash_from_tx(&data.cpfp_tx, 0, &data.parent_output)?,
        data,
    ));
    if let Some(refund_tx) = &data.cpfp_refund_tx {
        let cpfp_output = data.cpfp_tx.output.first().ok_or_else(|| {
            ServiceError::Generic("a node transaction has no outputs".to_string())
        })?;
        out.push((sighash_from_tx(refund_tx, 0, cpfp_output)?, data));
    }
    for child in &data.children {
        collect_unsigned(child, out)?;
    }
    Ok(())
}

fn to_creation_node(
    data: &NodeBuildData,
    shares: &mut impl Iterator<Item = SignedShare>,
) -> Result<crate::operator_rpc::CreationNode, ServiceError> {
    let mut signed = |tx: &Transaction| {
        shares
            .next()
            .map(|share| share.into_signing_job(&data.signing_public_key, tx))
            .ok_or_else(|| ServiceError::Generic("a transaction has no share".to_string()))
    };
    let node_tx_signing_job = Some(signed(&data.cpfp_tx)?);
    let refund_tx_signing_job = data.cpfp_refund_tx.as_ref().map(&mut signed).transpose()?;
    Ok(crate::operator_rpc::CreationNode {
        node_tx_signing_job,
        refund_tx_signing_job,
        children: data
            .children
            .iter()
            .map(|child| to_creation_node(child, shares))
            .collect::<Result<_, _>>()?,
        direct_node_tx_signing_job: None,
        direct_refund_tx_signing_job: None,
        // The operators sign the direct-from-CPFP refund only alongside a direct
        // refund, so the SSP sends this one without a share.
        direct_from_cpfp_refund_tx_signing_job: data.direct_from_cpfp_refund_tx.as_ref().map(
            |tx| UserSignedTxSigningJob {
                signing_public_key: data.signing_public_key.serialize().to_vec(),
                raw_tx: serialize(tx),
                ..Default::default()
            },
        ),
    })
}

fn collect_call_nodes(
    data: &NodeBuildData,
    finalized: &HashMap<Txid, TreeNode>,
    out: &mut CallNodes,
) -> Result<(), ServiceError> {
    let node = finalized
        .get(&data.cpfp_tx.compute_txid())
        .cloned()
        .ok_or_else(|| {
            ServiceError::Generic("the operators did not finalize every node".to_string())
        })?;
    if data.children.is_empty() {
        out.frontier.push(node);
        return Ok(());
    }
    out.branches.push(node);
    for child in &data.children {
        collect_call_nodes(child, finalized, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        fits_in_one_call, frontier_at_depth, nodes_needed, split_depth, truncate_at_depth,
    };
    use crate::tree::builder::{TreeBlueprint, build_tree_blueprint};

    const MAX_NODES: usize = 1000;

    fn uniform(leaves: usize) -> TreeBlueprint {
        build_tree_blueprint(vec![1; leaves], 2).expect("blueprint")
    }

    #[test]
    fn a_tree_within_the_limit_needs_no_split() {
        assert!(fits_in_one_call(&uniform(256), MAX_NODES));
    }

    #[test]
    fn the_limit_counts_branches_as_well_as_leaves() {
        let blueprint = uniform(512);
        assert_eq!(nodes_needed(&blueprint), 1023);
        assert!(!fits_in_one_call(&blueprint, MAX_NODES));
    }

    #[test]
    fn the_cut_is_the_shallowest_whose_subtrees_fit() {
        let blueprint = uniform(1024);
        let depth = split_depth(&blueprint, MAX_NODES);
        assert_eq!(depth, 2);

        assert!(fits_in_one_call(
            &truncate_at_depth(&blueprint, depth),
            MAX_NODES
        ));
        let frontier = frontier_at_depth(&blueprint, depth);
        assert_eq!(frontier.len(), 4);
        assert!(
            frontier
                .iter()
                .all(|subtree| fits_in_one_call(subtree, MAX_NODES))
        );
    }

    #[test]
    fn a_higher_limit_splits_the_same_tree_into_fewer_calls() {
        assert!(fits_in_one_call(&uniform(1024), 4000));
    }

    #[test]
    fn the_top_layer_ends_in_splits() {
        let blueprint = uniform(1024);
        let top = truncate_at_depth(&blueprint, 2);
        let frontier: Vec<&TreeBlueprint> = frontier_at_depth(&top, 2);
        assert_eq!(frontier.len(), 4);
        assert!(
            frontier
                .iter()
                .all(|node| matches!(node, TreeBlueprint::Split { value: 256 }))
        );
        assert_eq!(top.leaf_count(), 0);
        assert_eq!(nodes_needed(&top), 7);
    }

    #[test]
    fn a_cut_preserves_every_leaf() {
        let blueprint = uniform(1024);
        let frontier = frontier_at_depth(&blueprint, split_depth(&blueprint, MAX_NODES));
        let leaves: usize = frontier.iter().map(|subtree| subtree.leaf_count()).sum();
        assert_eq!(leaves, 1024);
        let value: u64 = frontier.iter().map(|subtree| subtree.value()).sum();
        assert_eq!(value, blueprint.value());
    }
}
