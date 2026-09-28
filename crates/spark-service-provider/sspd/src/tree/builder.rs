use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, absolute::LockTime,
    key::Secp256k1, secp256k1::PublicKey, transaction::Version,
};

use spark::{Network, services::ServiceError};

#[derive(Debug, Clone)]
pub enum TreeBlueprint {
    Leaf {
        value: u64,
    },
    Branch {
        value: u64,
        children: Vec<TreeBlueprint>,
    },
    /// A node whose subtree a later creation call builds.
    Split {
        value: u64,
    },
}

impl TreeBlueprint {
    pub fn value(&self) -> u64 {
        match self {
            TreeBlueprint::Leaf { value }
            | TreeBlueprint::Branch { value, .. }
            | TreeBlueprint::Split { value } => *value,
        }
    }

    #[cfg(test)]
    pub fn depth(&self) -> usize {
        match self {
            TreeBlueprint::Leaf { .. } | TreeBlueprint::Split { .. } => 0,
            TreeBlueprint::Branch { children, .. } => children
                .iter()
                .map(TreeBlueprint::depth)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        }
    }

    #[cfg(test)]
    pub fn max_children(&self) -> usize {
        match self {
            TreeBlueprint::Leaf { .. } | TreeBlueprint::Split { .. } => 0,
            TreeBlueprint::Branch { children, .. } => {
                let own = children.len();
                let child_max = children
                    .iter()
                    .map(TreeBlueprint::max_children)
                    .max()
                    .unwrap_or(0);
                own.max(child_max)
            }
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            TreeBlueprint::Leaf { .. } => 1,
            TreeBlueprint::Split { .. } => 0,
            TreeBlueprint::Branch { children, .. } => {
                children.iter().map(TreeBlueprint::leaf_count).sum()
            }
        }
    }

    pub fn leaf_values(&self) -> Vec<u64> {
        match self {
            TreeBlueprint::Leaf { value } => vec![*value],
            TreeBlueprint::Split { .. } => Vec::new(),
            TreeBlueprint::Branch { children, .. } => children
                .iter()
                .flat_map(TreeBlueprint::leaf_values)
                .collect(),
        }
    }

    pub fn children(&self) -> &[TreeBlueprint] {
        match self {
            TreeBlueprint::Leaf { .. } | TreeBlueprint::Split { .. } => &[],
            TreeBlueprint::Branch { children, .. } => children,
        }
    }

    pub fn is_leaf(&self) -> bool {
        matches!(self, TreeBlueprint::Leaf { .. })
    }
}

pub fn build_tree_blueprint(
    mut leaf_values: Vec<u64>,
    branch_factor: usize,
) -> Result<TreeBlueprint, ServiceError> {
    if leaf_values.is_empty() {
        return Err(ServiceError::Generic(
            "leaf_values must not be empty".to_string(),
        ));
    }

    if branch_factor < 2 {
        return Err(ServiceError::Generic(format!(
            "branch_factor must be at least 2, got {branch_factor}"
        )));
    }

    for &v in &leaf_values {
        if v == 0 || !v.is_power_of_two() {
            return Err(ServiceError::Generic(format!(
                "leaf value {v} is not a power of two"
            )));
        }
    }

    leaf_values.sort_unstable_by(|a, b| b.cmp(a));

    Ok(build_subtree(&leaf_values, branch_factor))
}

fn build_subtree(leaves: &[u64], branch_factor: usize) -> TreeBlueprint {
    if let [value] = leaves {
        return TreeBlueprint::Leaf { value: *value };
    }

    if leaves.len() <= branch_factor {
        let children: Vec<TreeBlueprint> = leaves
            .iter()
            .map(|&v| TreeBlueprint::Leaf { value: v })
            .collect();
        let value = children.iter().map(TreeBlueprint::value).sum();
        return TreeBlueprint::Branch { value, children };
    }

    let chunk_size = leaves.len().div_ceil(branch_factor);
    let children: Vec<TreeBlueprint> = leaves
        .chunks(chunk_size)
        .map(|chunk| build_subtree(chunk, branch_factor))
        .collect();
    let value = children.iter().map(TreeBlueprint::value).sum();

    TreeBlueprint::Branch { value, children }
}

pub fn create_branch_spark_tx(
    previous_output: OutPoint,
    sequence: Sequence,
    children: &[(Amount, ScriptBuf)],
    include_anchor: bool,
) -> Transaction {
    let mut outputs: Vec<TxOut> = children
        .iter()
        .map(|(value, script_pubkey)| TxOut {
            value: *value,
            script_pubkey: script_pubkey.clone(),
        })
        .collect();

    if include_anchor {
        outputs.push(ephemeral_anchor_output());
    }

    Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            sequence,
            ..Default::default()
        }],
        output: outputs,
    }
}

fn ephemeral_anchor_output() -> TxOut {
    TxOut {
        // Pay-to-anchor (P2A)
        script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        value: Amount::from_sat(0),
    }
}

pub fn p2tr_script_pubkey(verifying_key: &PublicKey, network: Network) -> ScriptBuf {
    let secp = Secp256k1::new();
    let addr = Address::p2tr(
        &secp,
        verifying_key.x_only_public_key().0,
        None,
        Into::<bitcoin::Network>::into(network),
    );
    addr.script_pubkey()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_tree_single_leaf() {
        let bp = build_tree_blueprint(vec![1024], 2).unwrap();
        assert!(bp.is_leaf());
        assert_eq!(bp.value(), 1024);
        assert_eq!(bp.leaf_count(), 1);
    }

    #[test]
    fn test_build_tree_two_leaves() {
        let bp = build_tree_blueprint(vec![512, 256], 2).unwrap();
        assert!(!bp.is_leaf());
        assert_eq!(bp.value(), 768);
        assert_eq!(bp.leaf_count(), 2);
        assert_eq!(bp.children().len(), 2);
    }

    #[test]
    fn test_build_tree_multiple_leaves_binary() {
        let bp = build_tree_blueprint(vec![8, 4, 1], 2).unwrap();
        assert_eq!(bp.value(), 13);
        assert_eq!(bp.leaf_count(), 3);

        let mut leaf_vals = bp.leaf_values();
        leaf_vals.sort_unstable();
        assert_eq!(leaf_vals, vec![1, 4, 8]);
    }

    #[test]
    fn test_build_tree_duplicate_denominations() {
        let bp = build_tree_blueprint(vec![64, 64, 32, 32, 16, 16], 2).unwrap();
        assert_eq!(bp.value(), 224);
        assert_eq!(bp.leaf_count(), 6);
    }

    #[test]
    fn test_build_tree_branch_factor_4() {
        let bp = build_tree_blueprint(vec![8, 4, 2, 1], 4).unwrap();
        assert_eq!(bp.value(), 15);
        assert_eq!(bp.leaf_count(), 4);
        assert_eq!(bp.children().len(), 4);
        for child in bp.children() {
            assert!(child.is_leaf());
        }
    }

    #[test]
    fn test_build_tree_branch_factor_wider_than_leaves() {
        let bp = build_tree_blueprint(vec![8, 4, 2], 8).unwrap();
        assert_eq!(bp.leaf_count(), 3);
        assert_eq!(bp.children().len(), 3);
    }

    #[test]
    fn test_build_tree_branch_factor_deeper() {
        let bp = build_tree_blueprint(vec![1, 1, 1, 1, 1, 1, 1, 1], 4).unwrap();
        assert_eq!(bp.leaf_count(), 8);
        assert_eq!(bp.children().len(), 4);
        for child in bp.children() {
            assert_eq!(child.leaf_count(), 2);
            assert_eq!(child.children().len(), 2);
        }
    }

    #[test]
    fn test_build_tree_empty_fails() {
        assert!(build_tree_blueprint(vec![], 2).is_err());
    }

    #[test]
    fn test_build_tree_branch_factor_1_fails() {
        assert!(build_tree_blueprint(vec![1], 1).is_err());
    }

    #[test]
    fn test_build_tree_non_power_of_two_fails() {
        assert!(build_tree_blueprint(vec![3], 2).is_err());
        assert!(build_tree_blueprint(vec![6], 2).is_err());
        assert!(build_tree_blueprint(vec![0], 2).is_err());
    }

    fn bp12(n: usize) -> TreeBlueprint {
        build_tree_blueprint(vec![1; n], 12).unwrap()
    }

    #[test]
    fn test_bf12_11_leaves_depth_1() {
        let bp = bp12(11);
        assert_eq!(bp.leaf_count(), 11);
        assert_eq!(bp.depth(), 1);
        assert_eq!(bp.children().len(), 11);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_12_leaves_depth_1() {
        let bp = bp12(12);
        assert_eq!(bp.leaf_count(), 12);
        assert_eq!(bp.depth(), 1);
        assert_eq!(bp.children().len(), 12);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_13_leaves_depth_2() {
        let bp = bp12(13);
        assert_eq!(bp.leaf_count(), 13);
        assert_eq!(bp.depth(), 2);
        assert!(bp.children().len() <= 12);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_143_leaves_depth_2() {
        let bp = bp12(143);
        assert_eq!(bp.leaf_count(), 143);
        assert_eq!(bp.depth(), 2);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_144_leaves_depth_2() {
        let bp = bp12(144);
        assert_eq!(bp.leaf_count(), 144);
        assert_eq!(bp.depth(), 2);
        assert_eq!(bp.children().len(), 12);
        for child in bp.children() {
            assert_eq!(child.children().len(), 12);
            for grandchild in child.children() {
                assert!(grandchild.is_leaf());
            }
        }
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_145_leaves_depth_3() {
        let bp = bp12(145);
        assert_eq!(bp.leaf_count(), 145);
        assert_eq!(bp.depth(), 3);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_1727_leaves_depth_3() {
        let bp = bp12(1727);
        assert_eq!(bp.leaf_count(), 1727);
        assert_eq!(bp.depth(), 3);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_1728_leaves_depth_3() {
        let bp = bp12(1728);
        assert_eq!(bp.leaf_count(), 1728);
        assert_eq!(bp.depth(), 3);
        assert_eq!(bp.children().len(), 12);
        for child in bp.children() {
            assert_eq!(child.children().len(), 12);
            for grandchild in child.children() {
                assert_eq!(grandchild.children().len(), 12);
                for leaf in grandchild.children() {
                    assert!(leaf.is_leaf());
                }
            }
        }
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_bf12_1729_leaves_depth_4() {
        let bp = bp12(1729);
        assert_eq!(bp.leaf_count(), 1729);
        assert_eq!(bp.depth(), 4);
        assert!(bp.max_children() <= 12);
    }

    #[test]
    fn test_branch_tx_output_count() {
        let outpoint = OutPoint::default();
        let children = vec![
            (Amount::from_sat(100), ScriptBuf::default()),
            (Amount::from_sat(200), ScriptBuf::default()),
        ];

        let tx = create_branch_spark_tx(outpoint, Sequence::ZERO, &children, false);
        assert_eq!(tx.output.len(), 2);

        let tx_with_anchor = create_branch_spark_tx(outpoint, Sequence::ZERO, &children, true);
        assert_eq!(tx_with_anchor.output.len(), 3);
        assert_eq!(tx_with_anchor.output[2].value.to_sat(), 0);
    }
}
