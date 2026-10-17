pub mod claimer;
pub mod repository;
pub mod select;

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bitcoin::secp256k1::PublicKey;
use spark::operator::OperatorPool;
use spark::services::{
    LeafKeyTweak, ServiceError, Transfer, TransferId, TransferService, TransferStatus, TransferType,
};
use spark::signer::Signer;
use spark::tree::TreeNode;
use thiserror::Error;
use tracing::{error, info};

use crate::graphql::types::RequestSwapInput;
use crate::in_flight::InFlight;
use crate::leaves::{self, IncomingTransfer, LeafSigningKeys};
use crate::pool::operator_view;
use crate::postgresql::{LeafStoreError, PostgresLeafStore};
use crate::wakeup::Wakeup;

use self::repository::{SwapDetail, SwapLeaf, SwapRecord};
use self::select::swap_denominations;

const SWAP_EXPIRY_DURATION: Duration = Duration::from_secs(2 * 60);

#[async_trait::async_trait]
pub trait SwapStore: Send + Sync {
    async fn insert_swap(&self, swap: &SwapRecord) -> Result<(), String>;

    async fn get_by_user_transfer_id(
        &self,
        user_transfer_id: &str,
    ) -> Result<Option<SwapRecord>, String>;

    async fn delete_swap(&self, swap_id: &str) -> Result<(), String>;

    async fn get_unclaimed_swaps(&self) -> Result<Vec<SwapRecord>, String>;

    async fn has_unclaimed_swaps(&self) -> Result<bool, String>;

    /// The most recent `limit` swaps, newest first.
    async fn list_swaps(&self, limit: u32) -> Result<Vec<SwapDetail>, String>;

    /// Returns the given transfers that are a swap's counter transfer.
    async fn counter_transfers_among(
        &self,
        transfer_ids: &[String],
    ) -> Result<HashSet<String>, String>;

    async fn record_outbound_leaves(
        &self,
        swap_id: &str,
        outbound_leaves: &[SwapLeaf],
    ) -> Result<(), String>;

    async fn record_inbound_leaves(
        &self,
        swap_id: &str,
        inbound_leaves: &[SwapLeaf],
    ) -> Result<(), String>;
}

#[derive(Error, Debug)]
pub enum SwapError {
    #[error("spark service error: {0}")]
    Service(#[from] ServiceError),

    #[error("leaf store error: {0}")]
    LeafStore(#[from] LeafStoreError),

    #[error("signer error: {0}")]
    Signer(#[from] spark::signer::SignerError),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("insufficient pool leaves for amount {amount}")]
    InsufficientPool { amount: u64 },

    #[error("transfer not found: {0}")]
    TransferNotFound(String),

    #[error("refused: {0}")]
    Refused(String),

    #[error("persistence error: {0}")]
    Persistence(String),
}

pub struct SwapService {
    leaf_store: Arc<PostgresLeafStore>,
    transfer_service: Arc<TransferService>,
    operator_pool: Arc<OperatorPool>,
    signer: Arc<dyn Signer>,
    network: spark::Network,
    key_resolver: Arc<dyn LeafSigningKeys>,
    swap_repo: Arc<dyn SwapStore>,
    requests: Arc<InFlight>,
    claim_wakeup: Wakeup,
    largest_denomination: u64,
}

pub struct SwapResult {
    pub swap_id: String,
    pub counter_transfer: Transfer,
    pub total_amount: u64,
    pub target_amount: u64,
    pub fee: u64,
}

impl SwapService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        leaf_store: Arc<PostgresLeafStore>,
        transfer_service: Arc<TransferService>,
        operator_pool: Arc<OperatorPool>,
        signer: Arc<dyn Signer>,
        network: spark::Network,
        key_resolver: Arc<dyn LeafSigningKeys>,
        swap_repo: Arc<dyn SwapStore>,
        requests: Arc<InFlight>,
        claim_wakeup: Wakeup,
        largest_denomination: u64,
    ) -> Self {
        Self {
            leaf_store,
            transfer_service,
            operator_pool,
            signer,
            network,
            key_resolver,
            swap_repo,
            requests,
            claim_wakeup,
            largest_denomination,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn request_swap(
        &self,
        input: &RequestSwapInput,
        caller: PublicKey,
    ) -> Result<SwapResult, SwapError> {
        let total_amount: u64 = input.total_amount_sats.0.try_into().map_err(|_| {
            SwapError::InvalidInput("total_amount_sats must be positive".to_string())
        })?;
        // The operators refuse a counter transfer whose value differs from the primary one's.
        if input.fee_sats.0 != 0 {
            return Err(SwapError::InvalidInput("fee_sats must be zero".to_string()));
        }
        let fee = 0;
        let amount_to_send = total_amount;

        let adaptor_public_key = PublicKey::from_str(&input.adaptor_pubkey.0)
            .map_err(|e| SwapError::InvalidInput(format!("invalid adaptor pubkey: {e}")))?;

        let primary_transfer_id =
            TransferId::from_str(&input.user_outbound_transfer_external_id.to_string())
                .map_err(|e| SwapError::InvalidInput(format!("invalid transfer id: {e}")))?;

        if let Some(existing) = self
            .swap_repo
            .get_by_user_transfer_id(&primary_transfer_id.to_string())
            .await
            .map_err(SwapError::Persistence)?
        {
            return self.existing_swap(existing, caller).await;
        }

        let incoming = IncomingTransfer::query(
            &self.operator_pool,
            self.signer.as_ref(),
            self.network,
            &primary_transfer_id,
        )
        .await?
        .ok_or_else(|| SwapError::TransferNotFound(primary_transfer_id.to_string()))?;
        let user_transfer = &incoming.transfer;
        let ssp_pubkey = spark::signer::derive_identity_public_key(self.signer.as_ref()).await?;
        if user_transfer.transfer_type != TransferType::PrimarySwapV3 {
            return Err(SwapError::Refused(
                "the transfer is not a primary swap".to_string(),
            ));
        }
        if !matches!(
            user_transfer.status,
            TransferStatus::SenderKeyTweakPending | TransferStatus::SenderInitiatedCoordinator
        ) {
            return Err(SwapError::Refused(format!(
                "the transfer cannot be swapped from status {}",
                user_transfer.status
            )));
        }
        if user_transfer.sender_identity_public_key != caller {
            return Err(SwapError::Refused(
                "the transfer is not from the caller".to_string(),
            ));
        }
        if user_transfer.receiver_identity_public_key != ssp_pubkey {
            return Err(SwapError::Refused(
                "the transfer is not addressed to the SSP".to_string(),
            ));
        }
        let primary_sum: u64 = user_transfer.leaves.iter().map(|l| l.leaf.value).sum();
        if primary_sum != total_amount {
            return Err(SwapError::Refused(format!(
                "the transfer does not carry total_amount_sats {total_amount}"
            )));
        }
        if !incoming
            .is_claimable(&self.transfer_service, self.signer.as_ref())
            .await
        {
            return Err(SwapError::Refused(
                "the SSP could not claim the transfer's leaves".to_string(),
            ));
        }

        info!(
            sender = %caller,
            amount = amount_to_send,
            "Processing swap request"
        );

        let targets = input
            .target_amount_sats
            .iter()
            .map(|t| u64::try_from(t.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                SwapError::InvalidInput("target_amount_sats must be non-negative".to_string())
            })?;
        let denominations = swap_denominations(&targets, amount_to_send, self.largest_denomination)
            .ok_or_else(|| {
                SwapError::InvalidInput(format!(
                    "target amounts sum exceeds amount to send {amount_to_send}"
                ))
            })?;

        // The service stores the swap before it reserves leaves and creates the
        // counter transfer. If the service stops partway, the claim loop can
        // settle the swap.
        let counter_transfer_id = TransferId::generate();
        let record = SwapRecord {
            id: uuid::Uuid::now_v7().to_string(),
            user_identity_public_key: caller.serialize().to_vec(),
            user_transfer_id: primary_transfer_id.clone(),
            counter_transfer_id: counter_transfer_id.clone(),
            total_amount_sats: i64::try_from(total_amount).unwrap_or(i64::MAX),
            target_amount_sats: i64::try_from(amount_to_send).unwrap_or(i64::MAX),
            fee_sats: i64::try_from(fee).unwrap_or(i64::MAX),
        };
        // The claim loop skips the swap while the service serves this request.
        let _request = self.requests.start(&record.id);
        self.swap_repo
            .insert_swap(&record)
            .await
            .map_err(SwapError::Persistence)?;
        let leaves = match self
            .leaf_store
            .reserve(&denominations, &counter_transfer_id)
            .await
        {
            Ok(Some(leaves)) => leaves,
            Ok(None) => {
                self.abandon(&record, &counter_transfer_id).await;
                return Err(SwapError::InsufficientPool {
                    amount: denominations.iter().sum(),
                });
            }
            Err(e) => {
                self.abandon(&record, &counter_transfer_id).await;
                return Err(e.into());
            }
        };
        info!(
            %counter_transfer_id,
            count = leaves.len(),
            values = ?leaves.iter().map(|l| l.value).collect::<Vec<_>>(),
            "Reserved pool leaves for swap"
        );
        let outbound: Vec<SwapLeaf> = leaves
            .iter()
            .map(|leaf| SwapLeaf {
                leaf_id: leaf.id.to_string(),
                value_sats: i64::try_from(leaf.value).unwrap_or(i64::MAX),
            })
            .collect();
        let prepared = match self
            .swap_repo
            .record_outbound_leaves(&record.id, &outbound)
            .await
        {
            Ok(()) => self.build_leaf_key_tweaks(&leaves).await,
            Err(e) => Err(SwapError::Persistence(e)),
        };
        let leaf_key_tweaks = match prepared {
            Ok(tweaks) => tweaks,
            Err(e) => {
                self.abandon(&record, &counter_transfer_id).await;
                return Err(e);
            }
        };

        // The operators can commit the counter transfer even when the service
        // gets an error back. The service then leaves the reserved leaves and
        // the stored swap to the claim loop.
        let expiry_time = SystemTime::now().checked_add(SWAP_EXPIRY_DURATION);
        let counter_transfer = crate::operator_rpc::counter_transfer(
            &self.transfer_service,
            &self.operator_pool.get_coordinator().client,
            &counter_transfer_id,
            &leaf_key_tweaks,
            &caller,
            &primary_transfer_id,
            &adaptor_public_key,
            expiry_time,
        )
        .await?;
        if let Err(e) = self.leaf_store.transferred(&counter_transfer_id).await {
            error!(%counter_transfer_id, "failed to mark the swap's leaves transferred: {e}");
        }

        self.claim_wakeup.wake();

        Ok(SwapResult {
            swap_id: record.id,
            counter_transfer,
            total_amount,
            target_amount: amount_to_send,
            fee,
        })
    }

    async fn existing_swap(
        &self,
        existing: SwapRecord,
        caller: PublicKey,
    ) -> Result<SwapResult, SwapError> {
        if existing.user_identity_public_key != caller.serialize().to_vec() {
            return Err(SwapError::Refused(
                "the transfer was already swapped".to_string(),
            ));
        }
        let Some(counter_transfer) = self
            .transfer_service
            .query_transfer(&existing.counter_transfer_id)
            .await?
        else {
            return Err(SwapError::Refused(
                "the swap for this transfer did not go through".to_string(),
            ));
        };
        Ok(SwapResult {
            swap_id: existing.id,
            counter_transfer,
            total_amount: existing.total_amount_sats.cast_unsigned(),
            target_amount: existing.target_amount_sats.cast_unsigned(),
            fee: existing.fee_sats.cast_unsigned(),
        })
    }

    /// If the function cannot release the leaves, it keeps the swap, so that the
    /// claim loop releases them later.
    async fn abandon(&self, record: &SwapRecord, counter_transfer_id: &TransferId) {
        if let Err(e) = operator_view::release(
            &self.operator_pool,
            self.network,
            &self.leaf_store,
            counter_transfer_id,
        )
        .await
        {
            error!(%counter_transfer_id, "failed to release a swap's leaves: {e}");
            return;
        }
        if let Err(e) = self.swap_repo.delete_swap(&record.id).await {
            error!(swap_id = %record.id, "could not delete a swap whose leaves were released: {e}");
        }
    }

    async fn build_leaf_key_tweaks(
        &self,
        leaves: &[TreeNode],
    ) -> Result<Vec<LeafKeyTweak>, SwapError> {
        leaves::leaf_key_tweaks(self.key_resolver.as_ref(), leaves)
            .await
            .map_err(|e| SwapError::InvalidInput(format!("key resolver error: {e}")))
    }
}
