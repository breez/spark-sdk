mod chain;
mod error;
mod exit_chain_resolver;
mod leaf_optimizer;
mod select_helper;
mod service;
mod store;

#[cfg(any(test, feature = "test-utils"))]
pub mod tests;

pub use chain::{assemble_exit_chains, chain_backs_exit, chain_reaches_root};
pub use error::TreeServiceError;
pub use exit_chain_resolver::ExitChainResolver;
pub use leaf_optimizer::*;
use platform_utils::tokio::sync::{broadcast, watch};
pub use select_helper::{
    select_leaves_by_minimum_amount, select_leaves_by_target_amounts, with_reserved_leaves,
};
use serde::{Deserialize, Serialize};
pub use service::SynchronousTreeService;
pub use store::{
    DEFAULT_MAX_CONCURRENT_RESERVATIONS, DEFAULT_RESERVATION_TIMEOUT, InMemoryTreeStore,
};
use tracing::trace;

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use bitcoin::{Sequence, Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;
use uuid::Uuid;

#[derive(PartialEq)]
pub struct Leaves {
    pub available: Vec<TreeNode>,
    pub not_available: Vec<TreeNode>,
    pub available_missing_from_operators: Vec<TreeNode>,
    /// Leaves reserved for payment operations - excluded from balance.
    pub reserved_for_payment: Vec<TreeNode>,
    /// Leaves reserved for swap - included in balance and not removed on refresh.
    pub reserved_for_swap: Vec<TreeNode>,
}

impl Leaves {
    pub fn available_balance(&self) -> u64 {
        self.available.iter().map(|leaf| leaf.value).sum()
    }
    pub fn missing_operators_balance(&self) -> u64 {
        self.available_missing_from_operators
            .iter()
            .map(|leaf| leaf.value)
            .sum()
    }
    pub fn payment_reserved_balance(&self) -> u64 {
        self.reserved_for_payment
            .iter()
            .map(|leaf| leaf.value)
            .sum()
    }
    pub fn swap_reserved_balance(&self) -> u64 {
        self.reserved_for_swap.iter().map(|leaf| leaf.value).sum()
    }
    /// Total balance including swap-reserved leaves but excluding
    /// payment-reserved leaves (since those are being spent).
    pub fn balance(&self) -> u64 {
        self.available_balance() + self.missing_operators_balance() + self.swap_reserved_balance()
    }

    /// Ids of every leaf in the set, across all buckets, deduplicated and
    /// ordered by id.
    pub fn leaf_ids(&self) -> Vec<TreeNodeId> {
        let mut ids: Vec<TreeNodeId> = self
            .available
            .iter()
            .chain(&self.not_available)
            .chain(&self.available_missing_from_operators)
            .chain(&self.reserved_for_payment)
            .chain(&self.reserved_for_swap)
            .map(|leaf| leaf.id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

/// The two public keys needed to confirm a stored leaf's ownership was already
/// verified, without loading the full node (and its transaction blobs). Its
/// signing pubkey is a deterministic function of the leaf id, so a stored leaf
/// whose verifying and signing-keyshare keys still match the coordinator's is
/// known-good: re-running the ownership check would only confirm it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedLeafKeys {
    pub verifying_public_key: PublicKey,
    pub signing_keyshare_public_key: PublicKey,
}

/// Projects the verified-leaf categories of `leaves` into the keys keyed by id.
/// The set mirrors `Leaves::balance` inputs plus payment-reserved leaves: every
/// category whose leaves passed the ownership check before being stored, and
/// deliberately excludes `not_available` (never verified).
pub fn verified_leaf_keys_from_leaves(leaves: &Leaves) -> HashMap<TreeNodeId, VerifiedLeafKeys> {
    leaves
        .available
        .iter()
        .chain(&leaves.available_missing_from_operators)
        .chain(&leaves.reserved_for_payment)
        .chain(&leaves.reserved_for_swap)
        .map(|leaf| {
            (
                leaf.id.clone(),
                VerifiedLeafKeys {
                    verifying_public_key: leaf.verifying_public_key,
                    signing_keyshare_public_key: leaf.signing_keyshare.public_key,
                },
            )
        })
        .collect()
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum TreeNodeStatus {
    /// Creating is the status of a tree node that is under creation.
    Creating,
    /// Available is the status of a tree node that is available.
    Available,
    /// FrozenByIssuer is the status of a tree node that is frozen by the issuer.
    FrozenByIssuer,
    /// TransferLocked is the status of a tree node that is transfer locked.
    TransferLocked,
    /// SplitLocked is the status of a tree node that is split locked.
    SplitLocked,
    /// Splitted is the status of a tree node that is splitted.
    Splitted,
    /// Aggregated is the status of a tree node that is aggregated, this is a terminal state.
    Aggregated,
    /// OnChain is the status of a tree node that is on chain, this is a terminal state.
    OnChain,
    /// Exited is the status of a tree node where the whole tree exited, this is a terminal state.
    Exited,
    /// AggregateLock is the status of a tree node that is aggregate locked.
    AggregateLock,
    /// Investigation is the status of a tree node that is investigated.
    Investigation,
    /// Lost is the status of a tree node that is in a unrecoverable bad state.
    Lost,
    /// Reimbursed is the status of a tree node that is reimbursed after LOST.
    Reimbursed,
    /// RenewLocked is the status of a tree node that is locked for renewal.
    RenewLocked,
    /// ParentExited is the status of a tree node whose parent is exiting,
    /// making this node not valid for transfer, timelock refresh, etc.
    ParentExited,
    /// Unknown is a status not yet recognized by this SDK version.
    Unknown,
}

impl std::fmt::Display for TreeNodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TreeNodeStatus::Creating => write!(f, "Creating"),
            TreeNodeStatus::Available => write!(f, "Available"),
            TreeNodeStatus::FrozenByIssuer => write!(f, "FrozenByIssuer"),
            TreeNodeStatus::TransferLocked => write!(f, "TransferLocked"),
            TreeNodeStatus::SplitLocked => write!(f, "SplitLocked"),
            TreeNodeStatus::Splitted => write!(f, "Splitted"),
            TreeNodeStatus::Aggregated => write!(f, "Aggregated"),
            TreeNodeStatus::OnChain => write!(f, "OnChain"),
            TreeNodeStatus::Exited => write!(f, "Exited"),
            TreeNodeStatus::AggregateLock => write!(f, "AggregateLock"),
            TreeNodeStatus::Investigation => write!(f, "Investigation"),
            TreeNodeStatus::Lost => write!(f, "Lost"),
            TreeNodeStatus::Reimbursed => write!(f, "Reimbursed"),
            TreeNodeStatus::RenewLocked => write!(f, "RenewLocked"),
            TreeNodeStatus::ParentExited => write!(f, "ParentExited"),
            TreeNodeStatus::Unknown => write!(f, "Unknown"),
        }
    }
}

impl From<&str> for TreeNodeStatus {
    fn from(s: &str) -> Self {
        match s {
            "CREATING" => TreeNodeStatus::Creating,
            "AVAILABLE" => TreeNodeStatus::Available,
            "FROZEN_BY_ISSUER" => TreeNodeStatus::FrozenByIssuer,
            "TRANSFER_LOCKED" => TreeNodeStatus::TransferLocked,
            "SPLIT_LOCKED" => TreeNodeStatus::SplitLocked,
            "SPLITTED" => TreeNodeStatus::Splitted,
            "AGGREGATED" => TreeNodeStatus::Aggregated,
            "ON_CHAIN" => TreeNodeStatus::OnChain,
            "EXITED" => TreeNodeStatus::Exited,
            "AGGREGATE_LOCK" => TreeNodeStatus::AggregateLock,
            "INVESTIGATION" => TreeNodeStatus::Investigation,
            "LOST" => TreeNodeStatus::Lost,
            "REIMBURSED" => TreeNodeStatus::Reimbursed,
            "RENEW_LOCKED" => TreeNodeStatus::RenewLocked,
            "PARENT_EXITED" => TreeNodeStatus::ParentExited,
            other => {
                tracing::warn!("Unrecognized TreeNodeStatus: {other}");
                TreeNodeStatus::Unknown
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    pub id: TreeNodeId,
    pub tree_id: String,
    pub value: u64,
    pub parent_node_id: Option<TreeNodeId>,
    pub node_tx: Transaction,
    // TODO: improve model to only allow empty refunds txs on expected cases
    pub refund_tx: Option<Transaction>,
    /// The direct transaction of the node, this transaction is for the watchtower to broadcast.
    pub direct_tx: Option<Transaction>,
    /// The refund transaction of the node, this transaction is to pay to the user.
    pub direct_refund_tx: Option<Transaction>,
    /// The refund transaction of the node, this transaction is to pay to the user.
    pub direct_from_cpfp_refund_tx: Option<Transaction>,
    /// This vout is the vout to spend the previous transaction, which is in the
    /// parent node.
    pub vout: u32,
    pub verifying_public_key: PublicKey,
    pub owner_identity_public_key: Option<PublicKey>,
    /// The signing keyshare information of the node on the SE side.
    pub signing_keyshare: SigningKeyshare,
    pub status: TreeNodeStatus,
}

impl TreeNode {
    fn node_sequence(&self) -> Sequence {
        self.node_tx.input[0].sequence
    }

    fn refund_sequence(&self) -> Result<Sequence, TreeServiceError> {
        Ok(self
            .refund_tx
            .as_ref()
            .ok_or(TreeServiceError::Generic("No refund tx".to_string()))?
            .input[0]
            .sequence)
    }

    pub fn needs_node_tx_renewed(&self) -> bool {
        let sequence_num = self.node_sequence().to_consensus_u32() as u16;
        trace!("Node tx sequence: {} node id: {}", sequence_num, self.id);
        sequence_num <= 100
    }

    pub fn needs_refund_tx_renewed(&self) -> Result<bool, TreeServiceError> {
        let sequence_num = self.refund_sequence()?.to_consensus_u32() as u16;
        trace!("Refund tx sequence: {} node id: {}", sequence_num, self.id);
        Ok(sequence_num <= 100)
    }

    pub fn is_zero_timelock(&self) -> bool {
        let sequence_num = self.node_sequence().to_consensus_u32() as u16;
        sequence_num == 0
    }

    pub fn direct_refund_tx(&self) -> Option<&Transaction> {
        // Zero-timelock nodes must not have a direct refund tx (SO enforces this).
        // During renew_zero_timelock, the direct refund tx is intentionally omitted,
        // so we must also omit it during transfers to avoid server rejection.
        match self.is_zero_timelock() {
            true => None,
            false => self.direct_tx.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TreeNodeId(String);

impl TreeNodeId {
    pub fn generate() -> Self {
        Self(Uuid::now_v7().to_string())
    }
}

impl std::fmt::Display for TreeNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TreeNodeId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("TreeNodeId cannot be empty".to_string());
        }
        Ok(TreeNodeId(s.to_string()))
    }
}

pub struct TreeNodeTransactionSequence {
    pub next_sequence: Sequence,
    pub needs_refresh: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SigningKeyshare {
    /// The identifiers of the owners of the keyshare.
    pub owner_identifiers: Vec<Identifier>,
    /// The threshold of the keyshare.
    pub threshold: u32,
    pub public_key: PublicKey,
}

pub type LeavesReservationId = String;

/// Result of a reservation attempt.
#[derive(Debug, Clone)]
pub enum ReserveResult {
    /// Reservation succeeded.
    Success(LeavesReservation),
    /// Not enough balance even with pending changes - fail immediately.
    InsufficientFunds,
    /// Not enough available balance but pending would help - caller should wait.
    WaitForPending {
        /// Amount that would satisfy this request.
        needed: u64,
        /// Current available balance.
        available: u64,
        /// Pending balance that will become available.
        pending: u64,
    },
}

impl ReserveResult {
    /// Converts `ReserveResult` into a `Result<LeavesReservation, TreeServiceError>`.
    ///
    /// - `Success` returns `Ok(reservation)`
    /// - `InsufficientFunds` returns `Err(TreeServiceError::InsufficientFunds)`
    /// - `WaitForPending` returns an error (unexpected after swap completion)
    pub fn into_result(self) -> Result<LeavesReservation, TreeServiceError> {
        match self {
            ReserveResult::Success(r) => Ok(r),
            ReserveResult::InsufficientFunds => Err(TreeServiceError::InsufficientFunds),
            ReserveResult::WaitForPending { .. } => Err(TreeServiceError::Generic(
                "Unexpected WaitForPending after swap".into(),
            )),
        }
    }
}

/// The purpose of a leaf reservation, which determines how the reserved
/// leaves are treated in balance calculations and whether they are removed on refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReservationPurpose {
    /// Leaves being used for a payment - excluded from balance since they
    /// are about to be spent.
    #[default]
    Payment,
    /// Leaves will be swapped. Included in balance since we will receive
    /// the same amount back. Not removed by set_leaves to ensure consistent balance calculation.
    Swap,
}

impl std::fmt::Display for ReservationPurpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReservationPurpose::Payment => write!(f, "Payment"),
            ReservationPurpose::Swap => write!(f, "Swap"),
        }
    }
}

impl std::str::FromStr for ReservationPurpose {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Payment" => Ok(ReservationPurpose::Payment),
            "Swap" => Ok(ReservationPurpose::Swap),
            _ => Err(format!("Unknown ReservationPurpose: {s}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LeavesReservation {
    pub id: LeavesReservationId,
    pub leaves: Vec<TreeNode>,
}

impl LeavesReservation {
    pub fn new(leaves: Vec<TreeNode>, id: LeavesReservationId) -> Self {
        Self { leaves, id }
    }

    pub fn sum(&self) -> u64 {
        self.leaves.iter().map(|leaf| leaf.value).sum()
    }
}

/// Default maximum time to wait for pending balance before giving up.
pub const DEFAULT_MAX_WAIT_FOR_PENDING: Duration = Duration::from_secs(60);

/// Options for leaf selection behavior.
#[derive(Clone, Debug)]
pub struct SelectLeavesOptions {
    /// Maximum time to wait for pending balance to become available.
    ///
    /// - `Duration::ZERO`: Return immediately without waiting
    /// - Other duration: Wait up to the specified duration
    ///
    /// Default: 60 seconds
    pub max_wait_for_pending: Duration,
}

impl Default for SelectLeavesOptions {
    fn default() -> Self {
        Self {
            max_wait_for_pending: DEFAULT_MAX_WAIT_FOR_PENDING,
        }
    }
}

impl SelectLeavesOptions {
    /// Create options for immediate return without waiting.
    pub fn no_wait() -> Self {
        Self {
            max_wait_for_pending: Duration::ZERO,
        }
    }
}

#[derive(Clone, Debug)]
pub enum TargetAmounts {
    AmountAndFee {
        amount_sats: u64,
        fee_sats: Option<u64>,
    },
    ExactDenominations {
        denominations: Vec<u64>,
    },
}

impl TargetAmounts {
    pub fn new_amount_and_fee(amount_sats: u64, fee_sats: Option<u64>) -> Self {
        Self::AmountAndFee {
            amount_sats,
            fee_sats,
        }
    }

    pub fn new_exact_denominations(denominations: Vec<u64>) -> Self {
        Self::ExactDenominations { denominations }
    }

    pub fn total_sats(&self) -> u64 {
        match self {
            Self::AmountAndFee {
                amount_sats,
                fee_sats,
            } => amount_sats + fee_sats.unwrap_or(0),
            Self::ExactDenominations { denominations } => denominations.iter().sum(),
        }
    }
}

pub struct TargetLeaves<L = TreeNode> {
    pub amount_leaves: Vec<L>,
    pub fee_leaves: Option<Vec<L>>,
}

impl<L> TargetLeaves<L> {
    pub fn new(amount_leaves: Vec<L>, fee_leaves: Option<Vec<L>>) -> Self {
        Self {
            amount_leaves,
            fee_leaves,
        }
    }
}

/// Minimal "leaf-shaped" interface used by the selection algorithms in
/// [`select_helper`]. Implementing this for a slim `(id, value)` projection
/// lets storage backends (e.g. `PostgresTreeStore`) run the same selection
/// without first deserializing every leaf's full `data` JSON.
pub trait LeafLike: Clone {
    type Id: Eq;
    fn leaf_id(&self) -> &Self::Id;
    fn leaf_value(&self) -> u64;
}

impl LeafLike for TreeNode {
    type Id = TreeNodeId;
    fn leaf_id(&self) -> &Self::Id {
        &self.id
    }
    fn leaf_value(&self) -> u64 {
        self.value
    }
}

/// A leaf together with its ancestor chain up to the root.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeafPedigree {
    pub leaf: TreeNode,
    pub ancestors: Vec<TreeNode>,
}

/// A low-level storage interface for managing tree nodes and leaf reservations.
///
/// `TreeStore` provides the fundamental storage operations for tree nodes, including
/// adding, retrieving, and updating leaves, as well as managing leaf reservations.
/// This trait abstracts the underlying storage mechanism, allowing for different
/// implementations such as in-memory storage, database storage, or distributed storage.
///
/// # Reservation System
///
/// The trait includes a reservation system that allows leaves to be temporarily
/// reserved for transactions, preventing double-spending while maintaining
/// transactional consistency.
#[macros::async_trait]
pub trait TreeStore: Send + Sync {
    /// Adds leaves to the pool. Re-adding an existing id overwrites the stored
    /// node: the operators are the source of truth for every field, including
    /// `value` and the verifying key. Chains are written by
    /// [`Self::store_ancestors`].
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError>;

    /// Stores the ancestors of `pedigrees`, leaving the leaf pool untouched.
    ///
    /// Each chain must run from its leaf's current parent to a root, which
    /// [`chain_reaches_root`] decides. A store takes a stored chain to be one that
    /// backs an exit and only looks for the parent, so a partial chain written
    /// here reads as exitable and produces transactions that cannot confirm.
    ///
    /// Unlike [`Self::add_leaves`], this neither inserts a leaf nor clears its spent
    /// marker, so a leaf spent since its pedigree was resolved stays spent, and a
    /// pedigree whose leaf the pool no longer holds is skipped: a chain is only ever
    /// removed with its leaf, so one stored for a leaf that is already gone would
    /// never be collected.
    async fn store_ancestors(&self, pedigrees: &[LeafPedigree]) -> Result<(), TreeServiceError>;

    /// Ids of the stored leaves that have no chain reaching their current parent,
    /// which is what [`Self::store_ancestors`] guarantees a stored chain does. See
    /// [`chain_backs_exit`], the same rule in Rust. A leaf that is itself a root
    /// needs no ancestors and is never reported.
    ///
    /// Returns ids only, but costs one lookup per stored leaf.
    async fn leaves_missing_exit_chains(&self) -> Result<Vec<TreeNodeId>, TreeServiceError>;

    /// Reconstructs the exit chains for many leaves at once, each as a
    /// [`LeafPedigree`] (the leaf plus its ancestors, nearest first), by walking
    /// `parent_node_id` up from each leaf. A leaf absent from the store is skipped,
    /// and a gap or cycle stops that walk and returns what it reached rather than
    /// erroring. Backends resolve the whole batch in a single query rather than one
    /// round-trip per leaf.
    async fn get_exit_chains(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, TreeServiceError>;

    /// Retrieves all leaves currently stored in the store.
    ///
    /// This method returns a complete snapshot of all tree nodes stored
    /// in the store, including both available and reserved leaves.
    ///
    /// # Returns
    ///
    /// * `Result<Leaves, TreeServiceError>` - A vector containing all stored
    ///   tree nodes if successful, or an error if the operation fails
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * Storage access fails
    /// * Data corruption is detected
    /// * Deserialization of stored data fails
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeStore, TreeNodeStatus, TreeServiceError};
    ///
    /// # async fn example(store: &dyn TreeStore) -> Result<(), TreeServiceError> {
    /// let all_leaves = store.get_leaves().await?;
    /// let available_count = all_leaves.available.iter()
    ///     .filter(|leaf| leaf.status == TreeNodeStatus::Available)
    ///     .count();
    /// println!("Found {} available leaves out of {}", available_count, all_leaves.available.len());
    /// # Ok(())
    /// # }
    /// ```
    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError>;

    /// Returns the wallet's spendable balance: the sum of leaf values that
    /// would be included in `Leaves::balance()` (available + missing-operators
    /// + swap-reserved). Default impl falls through to `get_leaves`; storage
    /// backends that can compute this server-side should override to skip the
    /// per-leaf fetch + deserialization round-trip.
    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        let leaves = self.get_leaves().await?;
        Ok(leaves.balance())
    }

    /// Returns, keyed by leaf id, the keys needed to tell whether a stored
    /// leaf's ownership was already verified (see [`VerifiedLeafKeys`]). Lets a
    /// refresh skip re-deriving a signer pubkey for leaves that are unchanged
    /// since they were stored. Default impl falls through to `get_leaves`;
    /// storage backends should override to project only these columns and skip
    /// loading full nodes (and their transaction blobs).
    async fn get_verified_leaf_keys(
        &self,
    ) -> Result<HashMap<TreeNodeId, VerifiedLeafKeys>, TreeServiceError> {
        Ok(verified_leaf_keys_from_leaves(&self.get_leaves().await?))
    }

    /// Returns the leaves no operator reports any more, which are kept for their
    /// exit chain alone and so belong to none of [`TreeStore::get_leaves`]'s
    /// buckets. This is the only way to reach them, and it exists for the check
    /// that decides whether one of them was really spent.
    async fn get_deleted_leaves(&self) -> Result<Vec<TreeNode>, TreeServiceError>;

    /// Removes `leaf_ids`, and any ancestor left unreachable, for good. Reserved
    /// for a leaf whose spend has been proven: anything else keeps its row, since
    /// a leaf that is still ours is exitable from its stored transactions alone.
    async fn remove_leaves(&self, leaf_ids: &[TreeNodeId]) -> Result<(), TreeServiceError>;

    /// Refreshes the leaf pool from `leaves`. A prior leaf absent from the set is
    /// marked rather than removed: it leaves the balance and the selection pool
    /// but keeps its row and the chain [`Self::store_ancestors`] wrote for it,
    /// because absence is not proof of a spend. A leaf added after
    /// `refresh_started_at` is kept unmarked, so a leaf added mid-refresh (e.g.
    /// from a payment) is not held back.
    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_started_at: platform_utils::time::SystemTime,
    ) -> Result<(), TreeServiceError>;

    /// Cancels a leaf reservation, replacing its leaves with the given keep-list.
    ///
    /// All leaves currently attached to this reservation are removed from the store.
    /// The reservation row is dropped. The supplied `leaves_to_keep` are then upserted
    /// into the available pool with no reservation.
    ///
    /// To preserve today's "release everything back to the pool" behavior, callers
    /// pass the original reservation leaves as `leaves_to_keep`. To drop part of the
    /// set (e.g. operator-locked leaves the caller has just verified), callers pass
    /// only the leaves they have confirmed are safe to make available. Only the
    /// leaves are passed: their ancestor chains stayed in the store while they were
    /// reserved, so a returned leaf is already offline-exitable.
    async fn cancel_reservation(
        &self,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError>;

    /// Finalizes a leaf reservation, marking the leaves as consumed and optionally adding new leaves to the main pool.
    ///
    /// This method permanently removes the reserved leaves from the store,
    /// indicating they have been successfully used in a transaction. Unlike
    /// cancellation, finalized leaves are not returned to the available pool.
    ///
    /// # Parameters
    ///
    /// * `id` - The unique reservation ID to finalize
    /// * `new_leaves` - Optional new leaves to add to the main pool.
    ///
    /// # Returns
    ///
    /// * `Result<(), TreeServiceError>` - Ok if the reservation was successfully
    ///   finalized, or an error if the operation fails
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * The reservation ID does not exist
    /// * The reservation has already been cancelled or finalized
    /// * Storage operation fails
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeStore, TargetAmounts, TreeServiceError, ReservationPurpose, ReserveResult};
    ///
    /// # async fn example(store: &dyn TreeStore) -> Result<(), TreeServiceError> {
    /// let target = TargetAmounts::new_amount_and_fee(100_000, Some(2_000));
    /// if let ReserveResult::Success(reservation) = store.try_reserve_leaves(Some(&target), false, ReservationPurpose::Payment).await? {
    ///     // After successfully using the leaves in a transaction
    ///     store.finalize_reservation(&reservation.id, None).await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError>;

    /// Attempts to reserve leaves. Returns `ReserveResult` indicating:
    /// - `Success`: reservation completed
    /// - `InsufficientFunds`: not enough even with pending
    /// - `WaitForPending`: should wait for balance change
    ///
    /// Automatically tracks pending balance when reserved > needed.
    ///
    /// # Parameters
    ///
    /// * `target_amounts` - Optional target amounts for selection
    /// * `exact_only` - If `true`, only exact matches are allowed
    /// * `purpose` - The purpose of the reservation
    ///
    /// # Returns
    ///
    /// * `Result<ReserveResult, TreeServiceError>` - The result of the reservation attempt
    async fn try_reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<ReserveResult, TreeServiceError>;

    async fn try_reserve_leaves_by_ids(
        &self,
        leaf_ids: &[TreeNodeId],
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError>;

    async fn try_select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeafSelection, TreeServiceError>;

    /// Returns the current time from the store's clock.
    ///
    /// For in-memory stores this returns `SystemTime::now()`. For database-backed
    /// stores this queries the database server time, avoiding clock skew between
    /// the application and database servers.
    async fn now(&self) -> Result<platform_utils::time::SystemTime, TreeServiceError>;

    /// Subscribe to balance change notifications.
    ///
    /// Returns a `watch::Receiver` that notifies when the balance changes.
    /// Callers can use `changed().await` to wait for balance changes.
    /// The value sent is not meaningful - only the notification matters.
    fn subscribe_balance_changes(&self) -> watch::Receiver<()>;

    /// Updates a reservation after a swap operation.
    ///
    /// This method is used when a swap has been performed and we need to
    /// update the reservation to use specific leaves from the swap output.
    /// The operation:
    /// 1. Updates the reservation to use the provided reserved_leaves
    /// 2. Adds the change_leaves to the available pool
    ///
    /// The reservation's permit is preserved - no new permit is needed.
    ///
    /// # Parameters
    ///
    /// * `reservation_id` - The ID of the existing reservation to update
    /// * `reserved_leaves` - The exact leaves to keep in the reservation
    /// * `change_leaves` - The leaves to add to the available pool (change from swap)
    ///
    /// Both sets carry their ancestors: swap outputs are fresh nodes whose chain
    /// is otherwise unstored, so without them the leaf could not be exited offline.
    ///
    /// # Returns
    ///
    /// * `Result<LeavesReservation, TreeServiceError>` - The updated reservation
    async fn update_reservation(
        &self,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError>;
}

pub enum LeafSelection {
    Exact(Vec<TreeNode>),
    SwapNeeded(Vec<TreeNode>),
}

#[macros::async_trait]
pub trait TreeService: Send + Sync {
    /// Notifies each time leaves reach the pool, for the work that has to follow
    /// them there: an exit chain to collect, a consolidation to consider.
    ///
    /// A prompt to go and look, not a record of what arrived. It carries no ids,
    /// and a listener that lags misses notifications, so what follows one has to
    /// read the pool for itself. A refresh cannot tell which of the leaves the
    /// operators reported are new and notifies for any of them.
    fn subscribe_leaves_added(&self) -> broadcast::Receiver<()>;

    /// Returns the total balance of all available leaves in the tree.
    ///
    /// This method calculates the sum of all leaf values that have a status of
    /// `TreeNodeStatus::Available`. It first retrieves all leaves from the local cache
    /// and filters out any that are not available before calculating the total.
    ///
    /// # Returns
    ///
    /// * `Result<u64, TreeServiceError>` - The total balance in satoshis if successful,
    ///   or an error if the operation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeServiceError};
    /// use spark::signer::Signer;
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Ensure the cache is up to date
    /// tree_service.refresh_leaves().await?;
    ///
    /// // Get the available balance
    /// let balance = tree_service.get_available_balance().await?;
    /// println!("Available balance: {} sats", balance);
    /// # Ok(())
    /// # }
    /// ```
    async fn get_available_balance(&self) -> Result<u64, TreeServiceError>;

    /// Lists all leaves from the local cache.
    ///
    /// This method retrieves the current set of tree nodes stored in the local state
    /// without making any network calls. To update the cache with the latest data
    /// from the server, call [`refresh_leaves`] first.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<TreeNode>, TreeServiceError>` - A vector of tree nodes representing
    ///   the leaves in the local cache, or an error if the operation fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeServiceError};
    /// use spark::signer::Signer;
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // First refresh to get the latest data
    /// tree_service.refresh_leaves().await?;
    ///
    /// // Then list the leaves
    /// let leaves = tree_service.list_leaves().await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn list_leaves(&self) -> Result<Leaves, TreeServiceError>;

    /// Fetches specific tree nodes by ID from the operators, optionally
    /// including each node's ancestors up to the root.
    async fn fetch_nodes(
        &self,
        node_ids: &[TreeNodeId],
        include_parents: bool,
    ) -> Result<Vec<TreeNode>, TreeServiceError>;

    /// Reads each leaf's exit chain (the leaf plus its ancestors up to the root)
    /// from the durable store, so an exit builds with the operators offline. A
    /// leaf whose stored chain does not reach the root comes back with a partial
    /// one; a leaf absent from the store is omitted.
    async fn load_exit_chains(
        &self,
        leaf_ids: &[TreeNodeId],
    ) -> Result<Vec<LeafPedigree>, TreeServiceError>;

    /// Persists each leaf's ancestor chain, leaving the leaf pool untouched, so a
    /// chain resolved after its leaf was stored completes it in place. Publishes
    /// an exit state change: a leaf that could not be exited offline before now
    /// can.
    async fn store_exit_chains(&self, pedigrees: &[LeafPedigree]) -> Result<(), TreeServiceError>;

    /// The same write, replaying a chain read back from a backup. Publishes
    /// nothing: the backup is where the chain came from, so it cannot be the
    /// thing that made the backup stale.
    async fn restore_exit_chains(&self, pedigrees: &[LeafPedigree])
    -> Result<(), TreeServiceError>;

    /// Ids of the stored leaves whose chain cannot back a unilateral exit.
    async fn leaves_missing_exit_chains(&self) -> Result<Vec<TreeNodeId>, TreeServiceError>;

    /// Pairs each leaf id with its ancestor chain, fetched from the operators in
    /// one `include_parents` query.
    ///
    /// Best-effort: a failed fetch (e.g. a sporadic network error) is not fatal and
    /// yields nothing rather than erroring, so a caller can try again later. A chain
    /// the operators cannot complete comes back partial, which a caller checks for.
    async fn fetch_pedigrees_from_operators(&self, leaf_ids: &[TreeNodeId]) -> Vec<LeafPedigree>;

    /// Refreshes the tree state by fetching the latest leaves from the server.
    ///
    /// This method clears the current local cache of leaves and fetches all available
    /// leaves from the coordinator, storing them in the local state. It handles pagination
    /// internally and will continue fetching until all leaves have been retrieved.
    ///
    /// # Returns
    ///
    /// * `Result<(), TreeServiceError>` - Ok if the refresh was successful, or an error
    ///   if any part of the operation fails.
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * Communication with the server fails
    /// * Deserialization of leaf data fails
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeServiceError};
    /// use spark::signer::Signer;
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Refresh the local cache with the latest leaves from the server
    /// tree_service.refresh_leaves().await?;
    ///
    /// // Now you can work with the updated leaves
    /// let leaves = tree_service.list_leaves().await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn refresh_leaves(&self) -> Result<(), TreeServiceError>;

    /// Removes the leaves kept only for their exit chain whose spend the
    /// operators can now prove, and returns how many went. Deliberately separate
    /// from [`TreeService::refresh_leaves`]: proving a spend costs a query per
    /// batch, and a refresh sits on the payment path.
    async fn purge_proven_spent_leaves(&self) -> Result<usize, TreeServiceError>;

    /// Inserts new leaves into the tree, each with the ancestors the caller
    /// already knows.
    ///
    /// A producer that already holds a leaf's chain (a deposit root has none; a
    /// claim can fetch it) passes it here so the store need not rediscover it.
    /// Any leaf a renewal reparents is completed against the coordinator.
    ///
    /// # Parameters
    ///
    /// * `leaves` - The leaves to insert.
    ///
    /// # Returns
    ///
    /// * `Result<Vec<TreeNode>, TreeServiceError>` - The inserted leaves (renewed
    ///   where a timelock required it), or an error if the operation fails.
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * The leaves contain invalid data
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TreeNode, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>, new_leaves: Vec<TreeNode>) -> Result<(), TreeServiceError> {
    /// // Insert leaves
    /// let result = tree_service.insert_leaves(new_leaves).await?;
    /// println!("Inserted {} leaves", result.len());
    /// # Ok(())
    /// # }
    /// ```
    async fn insert_leaves(&self, leaves: Vec<TreeNode>)
    -> Result<Vec<TreeNode>, TreeServiceError>;

    /// Stores leaves together with their ancestor chains exactly as given, with
    /// no timelock renewal and no operator round-trip. A leaf already stored
    /// keeps its place in the pool, and a chain given for it replaces its stored
    /// one wholesale, complete or not; an empty chain reads as unknown and
    /// leaves the stored one alone. Leaves not named are untouched.
    async fn restore_leaves(&self, leaves: &[LeafPedigree]) -> Result<(), TreeServiceError>;

    /// Selects and reserves leaves from the tree that match the specified target amounts.
    ///
    /// This method finds a combination of available leaves that can satisfy the target
    /// amounts for both the main amount and optional fee. The selected leaves are
    /// automatically reserved to prevent double-spending until the reservation is
    /// either finalized or cancelled.
    ///
    /// # Parameters
    ///
    /// * `target_amounts` - Optional target amounts specifying the desired amount and fee.
    ///   If `None`, all available leaves are selected.
    /// * `purpose` - The purpose of the reservation, which determines how reserved
    ///   leaves affect balance calculations. Use `Payment` for spending operations
    ///   and `Swap` for leaf reorganization.
    /// * `options` - Options controlling selection behavior, such as how long to wait
    ///   for pending balance to become available.
    ///
    /// # Returns
    ///
    /// * `Result<LeavesReservation, TreeServiceError>` - A reservation containing the
    ///   selected leaves and a unique reservation ID, or an error if no suitable
    ///   combination of leaves can be found.
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * No combination of leaves can satisfy the target amounts
    /// * Insufficient funds are available (or pending balance would be needed in no-wait mode)
    /// * The target amounts are invalid (e.g., zero or negative)
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TargetAmounts, ReservationPurpose, SelectLeavesOptions, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Select leaves for a specific amount with fee
    /// let target = TargetAmounts::new_amount_and_fee(100_000, Some(1_000)); // 100k sats + 1k fee
    /// let reservation = tree_service.select_leaves(Some(&target), ReservationPurpose::Payment, SelectLeavesOptions::default()).await?;
    /// println!("Reserved {} leaves with ID: {}", reservation.leaves.len(), reservation.id);
    ///
    /// # Ok(())
    /// # }
    /// ```
    async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        purpose: ReservationPurpose,
        options: SelectLeavesOptions,
    ) -> Result<LeavesReservation, TreeServiceError>;

    async fn reserve_leaves_by_ids(
        &self,
        leaf_ids: &[TreeNodeId],
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError>;

    /// Selects leaves for a deferred-signing send package without reserving
    /// them, renewing the node and refund timelocks of any selected leaf that
    /// needs it so the package is signed over up-to-date leaves. Renewal
    /// briefly reserves the leaves and persists the renewed data; when no
    /// renewal is needed the selection is read-only.
    async fn select_leaves_for_package(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeafSelection, TreeServiceError>;

    /// Cancels a leaf reservation and returns the reserved leaves to the available pool.
    ///
    /// This method releases a previously created reservation, making the reserved leaves
    /// available again for future selections. This is useful when a transaction fails
    /// or is aborted, and the reserved leaves should be returned to the pool.
    ///
    /// # Parameters
    ///
    /// * `reservation` - The reservation to cancel, including its current leaves
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * Storage operation fails
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TargetAmounts, ReservationPurpose, SelectLeavesOptions, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Create a reservation
    /// let target = TargetAmounts::new_amount_and_fee(50_000, None);
    /// let reservation = tree_service.select_leaves(Some(&target), ReservationPurpose::Payment, SelectLeavesOptions::default()).await?;
    ///
    /// // Later, if the transaction fails, cancel the reservation
    /// tree_service.cancel_reservation(reservation).await;
    /// println!("Reservation cancelled, leaves returned to pool");
    /// # Ok(())
    /// # }
    /// ```
    async fn cancel_reservation(
        &self,
        reservation: LeavesReservation,
    ) -> Result<(), TreeServiceError>;

    /// Finalizes a leaf reservation, marking the reserved leaves as consumed and optionally adding new leaves to the main pool.
    ///
    /// This method permanently removes the reserved leaves from the available pool,
    /// indicating that they have been successfully used in a transaction. Unlike
    /// [`cancel_reservation`], finalized leaves are not returned to the pool and
    /// are considered spent.
    ///
    /// # Parameters
    ///
    /// * `id` - The unique reservation ID returned from [`select_leaves`]
    /// * `new_leaves` - Optional new leaves to add to the main pool.
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * The reservation ID does not exist
    /// * The reservation has already been finalized
    /// * Storage operation fails
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TargetAmounts, ReservationPurpose, SelectLeavesOptions, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Create a reservation
    /// let target = TargetAmounts::new_amount_and_fee(75_000, Some(2_000));
    /// let reservation = tree_service.select_leaves(Some(&target), ReservationPurpose::Payment, SelectLeavesOptions::default()).await?;
    ///
    /// // After successfully using the leaves in a transaction, finalize the reservation
    /// tree_service.finalize_reservation(reservation.id, None).await;
    /// println!("Reservation finalized, leaves marked as spent");
    /// # Ok(())
    /// # }
    /// ```
    async fn finalize_reservation(
        &self,
        id: LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError>;
}
