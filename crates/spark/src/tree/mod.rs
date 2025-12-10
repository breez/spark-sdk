mod error;
mod select_helper;
mod service;
mod store;

pub use error::TreeServiceError;
pub use select_helper::{select_leaves_by_target_amounts, with_reserved_leaves};
use serde::{Deserialize, Serialize};
pub use service::SynchronousTreeService;
pub use store::InMemoryTreeStore;
use tracing::trace;

use std::str::FromStr;

use bitcoin::{Sequence, Transaction, secp256k1::PublicKey};
use frost_secp256k1_tr::Identifier;
use uuid::Uuid;

pub struct Leaves {
    pub available: Vec<TreeNode>,
    pub not_available: Vec<TreeNode>,
    pub available_missing_from_operators: Vec<TreeNode>,
    /// Leaves reserved for payment operations - excluded from balance.
    pub reserved_for_payment: Vec<TreeNode>,
    /// Leaves reserved for optimization - included in balance.
    pub reserved_for_optimization: Vec<TreeNode>,
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
    pub fn optimization_reserved_balance(&self) -> u64 {
        self.reserved_for_optimization
            .iter()
            .map(|leaf| leaf.value)
            .sum()
    }
    /// Total balance including optimization-reserved leaves but excluding
    /// payment-reserved leaves (since those are being spent).
    pub fn balance(&self) -> u64 {
        self.available_balance()
            + self.missing_operators_balance()
            + self.optimization_reserved_balance()
    }
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
}

impl std::str::FromStr for TreeNodeStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "CREATING" => Ok(TreeNodeStatus::Creating),
            "AVAILABLE" => Ok(TreeNodeStatus::Available),
            "FROZEN_BY_ISSUER" => Ok(TreeNodeStatus::FrozenByIssuer),
            "TRANSFER_LOCKED" => Ok(TreeNodeStatus::TransferLocked),
            "SPLIT_LOCKED" => Ok(TreeNodeStatus::SplitLocked),
            "SPLITTED" => Ok(TreeNodeStatus::Splitted),
            "AGGREGATED" => Ok(TreeNodeStatus::Aggregated),
            "ON_CHAIN" => Ok(TreeNodeStatus::OnChain),
            "EXITED" => Ok(TreeNodeStatus::Exited),
            "AGGREGATE_LOCK" => Ok(TreeNodeStatus::AggregateLock),
            "INVESTIGATION" => Ok(TreeNodeStatus::Investigation),
            "LOST" => Ok(TreeNodeStatus::Lost),
            "REIMBURSED" => Ok(TreeNodeStatus::Reimbursed),
            _ => Err(format!("Unknown TreeNodeStatus: {s}")),
        }
    }
}

#[derive(Debug, Clone)]
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
    pub owner_identity_public_key: PublicKey,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// The purpose of a leaf reservation, which determines how the reserved
/// leaves are treated in balance calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReservationPurpose {
    /// Leaves being used for a payment - excluded from balance since they
    /// are about to be spent.
    #[default]
    Payment,
    /// Leaves being reorganized by the optimizer - included in balance since
    /// the total value remains the same, just the denominations change.
    Optimization,
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

pub struct TargetLeaves {
    pub amount_leaves: Vec<TreeNode>,
    pub fee_leaves: Option<Vec<TreeNode>>,
}

impl TargetLeaves {
    pub fn new(amount_leaves: Vec<TreeNode>, fee_leaves: Option<Vec<TreeNode>>) -> Self {
        Self {
            amount_leaves,
            fee_leaves,
        }
    }
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
    /// Adds new leaves to the store without replacing existing ones.
    ///
    /// This method appends the provided leaves to the existing set of leaves
    /// in the store. If a leaf with the same ID already exists, the behavior
    /// is implementation-specific but typically the existing leaf is preserved.
    ///
    /// # Parameters
    ///
    /// * `leaves` - A slice of `TreeNode` objects to add to the store
    ///
    /// # Returns
    ///
    /// * `Result<(), TreeServiceError>` - Ok if the operation succeeds, or an error
    ///   if the leaves cannot be added
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * The leaves contain invalid data
    /// * Storage operation fails
    /// * Duplicate leaf IDs conflict with existing leaves
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeStore, TreeNode, TreeServiceError};
    ///
    /// # async fn example(store: &dyn TreeStore, new_leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
    /// // Add new leaves to the store
    /// store.add_leaves(new_leaves).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError>;

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

    /// Replaces all leaves in the store with the provided set.
    ///
    /// This method performs a complete replacement of the stored leaves,
    /// removing any existing leaves that are not in the provided set and
    /// adding or updating leaves as necessary. Reserved leaves may be
    /// updated with new data while maintaining their reservation status.
    ///
    /// # Parameters
    ///
    /// * `leaves` - A slice of `TreeNode` objects that will replace all existing leaves
    ///
    /// # Returns
    ///
    /// * `Result<(), TreeServiceError>` - Ok if the operation succeeds, or an error
    ///   if the leaves cannot be set
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * The leaves contain invalid data
    /// * Storage operation fails
    /// * Active reservations prevent the operation
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeStore, TreeNode, TreeServiceError};
    ///
    /// # async fn example(store: &dyn TreeStore, updated_leaves: &[TreeNode], missing_operators_leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
    /// // Replace all leaves with a new set
    /// store.set_leaves(updated_leaves, missing_operators_leaves).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
    ) -> Result<(), TreeServiceError>;

    /// Reserves leaves that match the specified target amounts.
    ///
    /// This method selects and reserves leaves from the available pool that
    /// can satisfy the target amounts. Reserved leaves are temporarily removed
    /// from the available pool to prevent double-spending until the reservation
    /// is either finalized or cancelled.
    ///
    /// # Parameters
    ///
    /// * `target_amounts` - Optional target amounts for selection. If `None`,
    ///   behavior is implementation-specific
    /// * `exact_only` - If `true`, only exact matches are allowed. If `false`,
    ///   approximate matches may be acceptable
    /// * `purpose` - The purpose of the reservation, which determines how
    ///   reserved leaves affect balance calculations
    ///
    /// # Returns
    ///
    /// * `Result<LeavesReservation, TreeServiceError>` - A reservation containing
    ///   the selected leaves and a unique reservation ID, or an error if no
    ///   suitable leaves can be found
    ///
    /// # Errors
    ///
    /// Returns a `TreeServiceError` if:
    /// * No leaves can satisfy the target amounts
    /// * Insufficient funds are available
    /// * The target amounts are invalid
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeStore, TargetAmounts, TreeServiceError, ReservationPurpose};
    ///
    /// # async fn example(store: &dyn TreeStore) -> Result<(), TreeServiceError> {
    /// let target = TargetAmounts::new_amount_and_fee(50_000, Some(1_000));
    /// let reservation = store.reserve_leaves(Some(&target), false, ReservationPurpose::Payment).await?;
    /// println!("Reserved {} leaves with ID: {}", reservation.leaves.len(), reservation.id);
    /// # Ok(())
    /// # }
    /// ```
    async fn reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError>;

    /// Cancels a leaf reservation and returns the leaves to the available pool.
    ///
    /// This method releases a previously created reservation, making the reserved
    /// leaves available again for future reservations. This is typically used
    /// when a transaction fails or is cancelled.
    ///
    /// # Parameters
    ///
    /// * `id` - The unique reservation ID to cancel
    ///
    /// # Returns
    ///
    /// * `Result<(), TreeServiceError>` - Ok if the reservation was successfully
    ///   cancelled, or an error if the operation fails
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
    /// use spark::tree::{TreeStore, TargetAmounts, TreeServiceError, ReservationPurpose};
    ///
    /// # async fn example(store: &dyn TreeStore) -> Result<(), TreeServiceError> {
    /// let target = TargetAmounts::new_amount_and_fee(25_000, None);
    /// let reservation = store.reserve_leaves(Some(&target), false, ReservationPurpose::Payment).await?;
    ///
    /// // Later, if the transaction is cancelled
    /// store.cancel_reservation(&reservation.id).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn cancel_reservation(&self, id: &LeavesReservationId) -> Result<(), TreeServiceError>;

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
    /// use spark::tree::{TreeStore, TargetAmounts, TreeServiceError, ReservationPurpose};
    ///
    /// # async fn example(store: &dyn TreeStore) -> Result<(), TreeServiceError> {
    /// let target = TargetAmounts::new_amount_and_fee(100_000, Some(2_000));
    /// let reservation = store.reserve_leaves(Some(&target), false, ReservationPurpose::Payment).await?;
    ///
    /// // After successfully using the leaves in a transaction
    /// store.finalize_reservation(&reservation.id, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError>;
}

#[macros::async_trait]
pub trait TreeService: Send + Sync {
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

    /// Inserts new leaves into the tree.
    ///
    /// This method adds the provided leaves to the tree state.
    ///
    /// # Parameters
    ///
    /// * `leaves` - A vector of `TreeNode` objects to insert into the tree
    ///
    /// # Returns
    ///
    /// * `Result<Vec<TreeNode>, TreeServiceError>` - The updated tree nodes after insertion,
    ///   or an error if the operation fails.
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
    ///   and `Optimization` for leaf reorganization.
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
    /// * Insufficient funds are available
    /// * The target amounts are invalid (e.g., zero or negative)
    ///
    /// # Examples
    ///
    /// ```
    /// use spark::tree::{TreeService, TargetAmounts, ReservationPurpose, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Select leaves for a specific amount with fee
    /// let target = TargetAmounts::new_amount_and_fee(100_000, Some(1_000)); // 100k sats + 1k fee
    /// let reservation = tree_service.select_leaves(Some(&target), ReservationPurpose::Payment).await?;
    /// println!("Reserved {} leaves with ID: {}", reservation.leaves.len(), reservation.id);
    ///   
    /// # Ok(())
    /// # }
    /// ```
    async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        purpose: ReservationPurpose,
    ) -> Result<LeavesReservation, TreeServiceError>;

    /// Cancels a leaf reservation and returns the reserved leaves to the available pool.
    ///
    /// This method releases a previously created reservation, making the reserved leaves
    /// available again for future selections. This is useful when a transaction fails
    /// or is aborted, and the reserved leaves should be returned to the pool.
    ///
    /// # Parameters
    ///
    /// * `id` - The unique reservation ID returned from [`select_leaves`]
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
    /// use spark::tree::{TreeService, TargetAmounts, ReservationPurpose, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Create a reservation
    /// let target = TargetAmounts::new_amount_and_fee(50_000, None);
    /// let reservation = tree_service.select_leaves(Some(&target), ReservationPurpose::Payment).await?;
    ///
    /// // Later, if the transaction fails, cancel the reservation
    /// tree_service.cancel_reservation(reservation.id).await;
    /// println!("Reservation cancelled, leaves returned to pool");
    /// # Ok(())
    /// # }
    /// ```
    async fn cancel_reservation(&self, id: LeavesReservationId) -> Result<(), TreeServiceError>;

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
    /// use spark::tree::{TreeService, TargetAmounts, ReservationPurpose, TreeServiceError};
    ///
    /// # async fn example(tree_service: Box<dyn TreeService>) -> Result<(), TreeServiceError> {
    /// // Create a reservation
    /// let target = TargetAmounts::new_amount_and_fee(75_000, Some(2_000));
    /// let reservation = tree_service.select_leaves(Some(&target), ReservationPurpose::Payment).await?;
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
