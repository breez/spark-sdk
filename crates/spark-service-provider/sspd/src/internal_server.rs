use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::Network;
use bitcoin::hashes::Hash;
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::chain::{ChainClient, ChainError, ChainRepository};
use crate::coop_exit::repository::{CoopExitRecord, CoopExitStore};
use crate::lightning::repository::{
    HoldInvoiceStatus, LightningReceiveRecord, LightningSendRecord, LightningStore,
    SendPaymentStatus,
};
use crate::pool::restock::{PoolEvent, RestockService};
use crate::static_deposit::repository::{StaticDepositClaimRecord, StaticDepositClaimStore};
use crate::swap::SwapStore;
use crate::swap::repository::SwapDetail;
use crate::wallet::SspWallet;

pub mod ssp_internal_api {
    #![allow(clippy::all, clippy::pedantic)]
    tonic::include_proto!("ssp_internal");
}

use ssp_internal_api::{
    BalanceRequest, BalanceResponse, CoopExit, FundingBroadcast, GetInfoRequest, GetInfoResponse,
    GetLightningRequestRequest, GetLightningRequestResponse, GetStaticDepositClaimRequest,
    GetStaticDepositClaimResponse, LeavesAvailable, LightningReceive, LightningSend,
    ListCoopExitsRequest, ListCoopExitsResponse, ListPendingLightningRequest,
    ListPendingLightningResponse, ListSwapsRequest, ListSwapsResponse, NewAddressRequest,
    NewAddressResponse, PoolEvent as ApiPoolEvent, PoolLeafCount, PoolStatusRequest,
    PoolStatusResponse, RequestRestockRequest, RequestRestockResponse, RestockDenomination,
    StaticDepositClaim, StopRequest, StopResponse, SubscribePoolEventsRequest, Swap, SwapLeaf,
    Utxo, UtxosRequest, UtxosResponse, onchain_wallet_server::OnchainWallet, pool_event,
    pool_server::Pool, ssp_manager_server::SspManager,
};

const DEFAULT_COOP_EXIT_LIMIT: u32 = 100;
const DEFAULT_SWAP_LIMIT: u32 = 100;

fn send_status_str(status: SendPaymentStatus) -> &'static str {
    match status {
        SendPaymentStatus::Pending => "pending",
        SendPaymentStatus::Succeeded => "succeeded",
        SendPaymentStatus::Failed => "failed",
    }
}

fn invoice_status_str(status: HoldInvoiceStatus) -> &'static str {
    match status {
        HoldInvoiceStatus::Pending => "pending",
        HoldInvoiceStatus::Settled => "settled",
        HoldInvoiceStatus::Cancelled => "cancelled",
        HoldInvoiceStatus::Failed => "failed",
    }
}

fn send_to_proto(record: &LightningSendRecord) -> LightningSend {
    LightningSend {
        id: record.id.clone(),
        user_identity_public_key: hex::encode(record.user_identity_public_key.serialize()),
        payment_hash: hex::encode(record.payment_hash.as_byte_array()),
        amount_sats: record.amount_sats,
        encoded_invoice: record.encoded_invoice.clone(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        fee_sats: record.fee_sats,
        user_transfer_id: record.user_transfer_id.to_string(),
        ln_payment_id: record.ln_payment_id.as_ref().map(|p| p.0.clone()),
        has_preimage: record.preimage.is_some(),
        payment_status: send_status_str(record.payment_status).to_string(),
        leaves_claimed: record.leaves_claimed,
        is_complete: record.is_complete(),
    }
}

fn swap_leaf_to_proto(leaf: &crate::swap::repository::SwapLeaf) -> SwapLeaf {
    SwapLeaf {
        leaf_id: leaf.leaf_id.clone(),
        value_sats: leaf.value_sats.unsigned_abs(),
    }
}

fn swap_to_proto(detail: &SwapDetail) -> Swap {
    Swap {
        id: detail.swap.id.clone(),
        user_transfer_id: detail.swap.user_transfer_id.clone(),
        total_amount_sats: detail.swap.total_amount_sats.unsigned_abs(),
        target_amount_sats: detail.swap.target_amount_sats.unsigned_abs(),
        fee_sats: detail.swap.fee_sats.unsigned_abs(),
        outbound: detail.outbound.iter().map(swap_leaf_to_proto).collect(),
        inbound: detail.inbound.iter().map(swap_leaf_to_proto).collect(),
    }
}

fn coop_exit_to_proto(record: &CoopExitRecord) -> CoopExit {
    CoopExit {
        id: record.id.clone(),
        user_identity_public_key: hex::encode(record.user_identity_public_key.serialize()),
        withdrawal_address: record.withdrawal_address.clone(),
        amount_sats: record.amount_sats,
        fee_sats: record.fee_sats,
        coop_exit_txid: record.coop_exit_txid.clone(),
        user_transfer_id: record.user_transfer_id.to_string(),
        completed: record.completed,
        broadcast_txid: record.broadcast_txid.clone(),
        leaves_claimed: record.leaves_claimed,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn static_deposit_claim_to_proto(record: &StaticDepositClaimRecord) -> StaticDepositClaim {
    StaticDepositClaim {
        id: record.id.clone(),
        user_identity_public_key: hex::encode(record.user_identity_public_key.serialize()),
        txid: record.txid.clone(),
        vout: record.vout,
        credit_amount_sats: record.credit_amount_sats,
        is_instant: record.is_instant,
        transfer_id: record.transfer_id.clone(),
        spend_broadcast_txid: record.spend_broadcast_txid.clone(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        spend_confirmed: record.spend_confirmed,
    }
}

fn receive_to_proto(record: &LightningReceiveRecord) -> LightningReceive {
    LightningReceive {
        id: record.id.clone(),
        user_identity_public_key: hex::encode(record.user_identity_public_key.serialize()),
        payment_hash: hex::encode(record.payment_hash.as_byte_array()),
        amount_sats: record.amount_sats,
        encoded_invoice: record.encoded_invoice.clone(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        transfer_id: record.transfer_id.as_ref().map(ToString::to_string),
        transfer_amount_sats: record.transfer_amount_sats,
        has_preimage: record.preimage.is_some(),
        invoice_status: invoice_status_str(record.invoice_status).to_string(),
        expires_at: record.expires_at.to_rfc3339(),
        memo: record.memo.clone(),
    }
}

pub struct ServerParams<C, R>
where
    C: ChainClient,
    R: ChainRepository,
{
    pub chain_client: Arc<C>,
    pub network: Network,
    pub token: CancellationToken,
    pub wallet: Arc<SspWallet<R>>,
    pub lightning_store: Arc<dyn LightningStore>,
    pub coop_exit_store: Arc<dyn CoopExitStore>,
    pub static_deposit_store: Arc<dyn StaticDepositClaimStore>,
    pub swap_store: Arc<dyn SwapStore>,
}

pub struct Server<C>
where
    C: ChainClient,
{
    chain_client: Arc<C>,
    network: Network,
    token: CancellationToken,
    lightning_store: Arc<dyn LightningStore>,
    coop_exit_store: Arc<dyn CoopExitStore>,
    static_deposit_store: Arc<dyn StaticDepositClaimStore>,
    swap_store: Arc<dyn SwapStore>,
}

impl<C> Server<C>
where
    C: ChainClient,
{
    pub fn new<R: ChainRepository>(params: &ServerParams<C, R>) -> Self {
        Self {
            chain_client: Arc::clone(&params.chain_client),
            network: params.network,
            token: params.token.clone(),
            lightning_store: Arc::clone(&params.lightning_store),
            coop_exit_store: Arc::clone(&params.coop_exit_store),
            static_deposit_store: Arc::clone(&params.static_deposit_store),
            swap_store: Arc::clone(&params.swap_store),
        }
    }
}

pub struct WalletServer<R: ChainRepository> {
    wallet: Arc<SspWallet<R>>,
}

impl<R: ChainRepository> WalletServer<R> {
    pub fn new<C: ChainClient>(params: &ServerParams<C, R>) -> Self {
        Self {
            wallet: Arc::clone(&params.wallet),
        }
    }
}

#[tonic::async_trait]
impl<C> SspManager for Server<C>
where
    C: ChainClient + Send + Sync + 'static,
{
    async fn get_info(
        &self,
        _request: tonic::Request<GetInfoRequest>,
    ) -> Result<tonic::Response<GetInfoResponse>, tonic::Status> {
        let block_height = self
            .chain_client
            .get_blockheight()
            .await
            .map_err(|e: ChainError| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(GetInfoResponse {
            block_height,
            network: self.network.to_string(),
        }))
    }

    async fn stop(
        &self,
        _request: tonic::Request<StopRequest>,
    ) -> Result<tonic::Response<StopResponse>, tonic::Status> {
        info!("stop requested via internal API");
        self.token.cancel();
        Ok(tonic::Response::new(StopResponse {}))
    }

    async fn get_lightning_request(
        &self,
        request: tonic::Request<GetLightningRequestRequest>,
    ) -> Result<tonic::Response<GetLightningRequestResponse>, tonic::Status> {
        let id = request.into_inner().id;
        if let Some(send) = self
            .lightning_store
            .get_send(&id)
            .await
            .map_err(tonic::Status::internal)?
        {
            return Ok(tonic::Response::new(GetLightningRequestResponse {
                send: Some(send_to_proto(&send)),
                receive: None,
            }));
        }
        let receive = self
            .lightning_store
            .get_receive(&id)
            .await
            .map_err(tonic::Status::internal)?;
        Ok(tonic::Response::new(GetLightningRequestResponse {
            send: None,
            receive: receive.as_ref().map(receive_to_proto),
        }))
    }

    async fn list_pending_lightning(
        &self,
        _request: tonic::Request<ListPendingLightningRequest>,
    ) -> Result<tonic::Response<ListPendingLightningResponse>, tonic::Status> {
        let sends = self
            .lightning_store
            .pending_sends()
            .await
            .map_err(tonic::Status::internal)?;
        let receives = self
            .lightning_store
            .pending_receives()
            .await
            .map_err(tonic::Status::internal)?;
        Ok(tonic::Response::new(ListPendingLightningResponse {
            sends: sends.iter().map(send_to_proto).collect(),
            receives: receives.iter().map(receive_to_proto).collect(),
        }))
    }

    async fn list_coop_exits(
        &self,
        request: tonic::Request<ListCoopExitsRequest>,
    ) -> Result<tonic::Response<ListCoopExitsResponse>, tonic::Status> {
        let limit = match request.into_inner().limit {
            0 => DEFAULT_COOP_EXIT_LIMIT,
            limit => limit,
        };
        let records = self
            .coop_exit_store
            .list(limit)
            .await
            .map_err(tonic::Status::internal)?;
        Ok(tonic::Response::new(ListCoopExitsResponse {
            coop_exits: records.iter().map(coop_exit_to_proto).collect(),
        }))
    }

    async fn list_swaps(
        &self,
        request: tonic::Request<ListSwapsRequest>,
    ) -> Result<tonic::Response<ListSwapsResponse>, tonic::Status> {
        let limit = match request.into_inner().limit {
            0 => DEFAULT_SWAP_LIMIT,
            limit => limit,
        };
        let details = self
            .swap_store
            .list_swaps(limit)
            .await
            .map_err(tonic::Status::internal)?;
        Ok(tonic::Response::new(ListSwapsResponse {
            swaps: details.iter().map(swap_to_proto).collect(),
        }))
    }

    async fn get_static_deposit_claim(
        &self,
        request: tonic::Request<GetStaticDepositClaimRequest>,
    ) -> Result<tonic::Response<GetStaticDepositClaimResponse>, tonic::Status> {
        let request = request.into_inner();
        let claim = self
            .static_deposit_store
            .get_by_utxo(&request.txid, request.vout)
            .await
            .map_err(tonic::Status::internal)?;
        Ok(tonic::Response::new(GetStaticDepositClaimResponse {
            claim: claim.as_ref().map(static_deposit_claim_to_proto),
        }))
    }
}

#[tonic::async_trait]
impl<R> OnchainWallet for WalletServer<R>
where
    R: ChainRepository + Send + Sync + 'static,
{
    async fn new_address(
        &self,
        _request: tonic::Request<NewAddressRequest>,
    ) -> Result<tonic::Response<NewAddressResponse>, tonic::Status> {
        let (address, index) = self
            .wallet
            .onchain
            .next_address()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(NewAddressResponse {
            address: address.to_string(),
            index,
        }))
    }

    async fn balance(
        &self,
        _request: tonic::Request<BalanceRequest>,
    ) -> Result<tonic::Response<BalanceResponse>, tonic::Status> {
        let utxos = self
            .wallet
            .onchain
            .list_utxos()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let confirmed_sats = utxos.iter().map(|u| u.value).sum();
        #[allow(clippy::cast_possible_truncation)]
        let utxo_count = utxos.len() as u32;
        Ok(tonic::Response::new(BalanceResponse {
            confirmed_sats,
            utxo_count,
        }))
    }

    async fn utxos(
        &self,
        _request: tonic::Request<UtxosRequest>,
    ) -> Result<tonic::Response<UtxosResponse>, tonic::Status> {
        let utxos = self
            .wallet
            .onchain
            .list_utxos()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(UtxosResponse {
            utxos: utxos
                .into_iter()
                .map(|u| Utxo {
                    txid: u.outpoint.txid.to_string(),
                    vout: u.outpoint.vout,
                    value: u.value,
                    block_height: u.block_height,
                    address: u.address.to_string(),
                })
                .collect(),
        }))
    }
}

pub struct PoolServer {
    restock: Arc<RestockService>,
    tree_store: Arc<dyn spark::tree::TreeStore>,
    denominations: Vec<u64>,
    /// Ends the event streams at shutdown: the server does not stop while a
    /// stream is open.
    token: CancellationToken,
}

impl PoolServer {
    pub fn new(
        restock: Arc<RestockService>,
        tree_store: Arc<dyn spark::tree::TreeStore>,
        denominations: Vec<u64>,
        token: CancellationToken,
    ) -> Self {
        Self {
            restock,
            tree_store,
            denominations,
            token,
        }
    }
}

fn to_restock_denominations(pending: HashMap<u64, u32>) -> Vec<RestockDenomination> {
    let mut out: Vec<RestockDenomination> = pending
        .into_iter()
        .map(|(denomination_sats, count)| RestockDenomination {
            denomination_sats,
            count,
        })
        .collect();
    out.sort_by_key(|d| d.denomination_sats);
    out
}

#[tonic::async_trait]
impl Pool for PoolServer {
    async fn request_restock(
        &self,
        request: Request<RequestRestockRequest>,
    ) -> Result<Response<RequestRestockResponse>, Status> {
        let denominations = request.into_inner().denominations;
        if let Some(unstocked) = denominations
            .iter()
            .find(|d| !self.denominations.contains(&d.denomination_sats))
        {
            return Err(Status::invalid_argument(format!(
                "the pool does not stock {} sat leaves",
                unstocked.denomination_sats
            )));
        }
        for denomination in denominations {
            self.restock
                .request(denomination.denomination_sats, denomination.count);
        }
        Ok(Response::new(RequestRestockResponse {
            pending: to_restock_denominations(self.restock.pending()),
        }))
    }

    async fn pool_status(
        &self,
        _request: Request<PoolStatusRequest>,
    ) -> Result<Response<PoolStatusResponse>, Status> {
        let leaves = self
            .tree_store
            .get_leaves()
            .await
            .map_err(|e| Status::internal(format!("could not read the leaf pool: {e}")))?;
        let mut counts: HashMap<u64, u32> = HashMap::new();
        let mut available_sats: u64 = 0;
        for leaf in &leaves.available {
            let count = counts.entry(leaf.value).or_insert(0);
            *count = count.saturating_add(1);
            available_sats = available_sats.saturating_add(leaf.value);
        }
        let mut available: Vec<PoolLeafCount> = counts
            .into_iter()
            .map(|(denomination_sats, count)| PoolLeafCount {
                denomination_sats,
                count,
            })
            .collect();
        available.sort_by_key(|c| c.denomination_sats);
        Ok(Response::new(PoolStatusResponse {
            available,
            available_sats,
            pending_restock: to_restock_denominations(self.restock.pending()),
        }))
    }

    type SubscribePoolEventsStream = ReceiverStream<Result<ApiPoolEvent, Status>>;

    async fn subscribe_pool_events(
        &self,
        _request: Request<SubscribePoolEventsRequest>,
    ) -> Result<Response<Self::SubscribePoolEventsStream>, Status> {
        let mut events = self.restock.subscribe();
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_STREAM_CAPACITY);
        let token = self.token.clone();
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    () = token.cancelled() => return,
                    event = events.recv() => event,
                };
                match event {
                    Ok(event) => {
                        if tx.send(Ok(api_event(event))).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        warn!("pool event subscriber lagged, missed {missed}");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

const EVENT_STREAM_CAPACITY: usize = 64;

fn api_event(event: PoolEvent) -> ApiPoolEvent {
    match event {
        PoolEvent::FundingBroadcast {
            txid,
            total_sats,
            denominations,
        } => ApiPoolEvent {
            event: Some(pool_event::Event::FundingBroadcast(FundingBroadcast {
                txid,
                total_sats,
                denominations,
            })),
        },
        PoolEvent::LeavesAvailable {
            deposit_address,
            denominations,
        } => ApiPoolEvent {
            event: Some(pool_event::Event::LeavesAvailable(LeavesAvailable {
                deposit_address,
                denominations,
            })),
        },
    }
}
