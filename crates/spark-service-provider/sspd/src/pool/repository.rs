use bitcoin::{OutPoint, Transaction};
use spark::tree::TreeNode;

#[derive(Debug, Clone)]
pub struct DepositTree {
    /// The funding output the tree's root spends.
    pub outpoint: OutPoint,
    pub deposit_address: String,
    pub denomination: u64,
    pub leaf_count: u32,
}

#[derive(Debug, Clone)]
pub struct TreeNodes {
    pub leaves: Vec<TreeNode>,
    pub branches: Vec<TreeNode>,
}

/// A funding transaction the SSP built, with its trees not yet in the pool.
#[derive(Debug, Clone)]
pub struct DepositTx {
    pub tx: Transaction,
    pub fee_sats: u64,
    pub stored_height: u64,
    pub latest_bump: Option<FundingBump>,
    pub trees: Vec<DepositTree>,
}

/// A child spending a funding transaction's change so the two together pay a higher
/// fee rate. Replacing the funding transaction instead would change the txid its
/// trees were created against.
#[derive(Debug, Clone)]
pub struct FundingBump {
    pub tx: Transaction,
    pub fee_sats: u64,
    pub height: u64,
}
