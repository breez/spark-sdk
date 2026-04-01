pub mod api;
pub mod config;
pub mod error;
pub mod events;
pub mod evm;
pub mod keys;
pub mod models;
pub mod recover;
pub mod store;
pub mod swap;

use std::sync::Arc;

use platform_utils::DefaultHttpClient;
use platform_utils::tokio::sync::mpsc;

pub use config::*;
pub use error::BoltzError;
pub use events::{BoltzEventListener, BoltzSwapEvent, EventEmitter};
pub use keys::EvmKeyManager;
pub use models::*;
pub use store::{BoltzStorage, MemoryBoltzStorage};

use api::BoltzApiClient;
use api::ws::SwapStatusSubscriber;
use evm::alchemy::AlchemyGasClient;
use evm::oft::OftDeployments;
use evm::provider::EvmProvider;
use evm::signing::EvmSigner;
use swap::manager::SwapManager;
use swap::reverse::ReverseSwapExecutor;

/// Top-level Boltz service facade.
///
/// Two-step swap flow:
/// - `prepare_reverse_swap` — pure quote, no side effects
/// - `create_reverse_swap` — commit to swap, get invoice; the swap is
///   automatically monitored and progressed to completion in the background
///
/// Call `start()` after construction to resume any active swaps from storage.
/// Register a `BoltzEventListener` to receive swap status updates.
pub struct BoltzService {
    executor: Arc<ReverseSwapExecutor>,
    swap_manager: SwapManager,
    event_emitter: Arc<EventEmitter>,
    ws_subscriber: Arc<SwapStatusSubscriber>,
}

impl BoltzService {
    /// Construct from config, seed bytes, and a store implementation.
    pub async fn new(
        config: BoltzConfig,
        seed: &[u8],
        store: Arc<dyn BoltzStorage>,
    ) -> Result<Self, BoltzError> {
        let key_manager = EvmKeyManager::from_seed(seed)?;

        // Derive gas signer for Alchemy
        let chain_id_u32: u32 = config
            .chain_id
            .try_into()
            .map_err(|_| BoltzError::Generic("Chain ID overflow".to_string()))?;
        let gas_key_pair = key_manager.derive_gas_signer(chain_id_u32)?;
        let gas_signer = EvmSigner::new(&gas_key_pair, config.chain_id);

        // Each component gets its own DefaultHttpClient. Instances are cheap
        // (no shared connection pool), so sharing via Arc is not worth the
        // signature churn.
        let api_client = BoltzApiClient::new(&config, Box::new(DefaultHttpClient::new(None)));

        // Create the global WS channel and subscriber.
        let (ws_tx, ws_rx) = mpsc::channel(256);
        let ws_subscriber = Arc::new(SwapStatusSubscriber::connect(&config.ws_url(), ws_tx).await?);

        let alchemy_client = AlchemyGasClient::new(
            &config.alchemy_config,
            Box::new(DefaultHttpClient::new(None)),
            gas_signer,
        );

        let evm_provider = EvmProvider::new(
            config.arbitrum_rpc_url.clone(),
            Box::new(DefaultHttpClient::new(None)),
        );

        // OFT deployments are fetched once and cached for the service lifetime.
        // They change rarely; a service restart picks up any updates.
        let oft_deployments =
            OftDeployments::fetch(&DefaultHttpClient::new(None), &config.oft_deployments_url)
                .await?;

        // Fetch contract addresses from the Boltz API, matching by chain ID
        let contracts = api_client.get_contracts().await?;
        let erc20swap_address = contracts
            .0
            .values()
            .find(|c| c.network.chain_id == config.chain_id)
            .map(|c| c.swap_contracts.erc20_swap.clone())
            .ok_or_else(|| BoltzError::Api {
                reason: format!(
                    "Chain ID {} not found in contracts response",
                    config.chain_id,
                ),
                code: None,
            })?;

        let executor = Arc::new(ReverseSwapExecutor::new(
            api_client,
            key_manager,
            alchemy_client,
            evm_provider,
            oft_deployments,
            store,
            config,
            erc20swap_address,
        ));

        let event_emitter = Arc::new(EventEmitter::new());

        let swap_manager = SwapManager::start(
            executor.clone(),
            event_emitter.clone(),
            ws_subscriber.clone(),
            ws_rx,
        );

        Ok(Self {
            executor,
            swap_manager,
            event_emitter,
            ws_subscriber,
        })
    }

    /// Load and resume all active (non-terminal) swaps from storage.
    /// Call once after construction to pick up swaps from previous runs.
    pub async fn resume_swaps(&self) -> Result<Vec<String>, BoltzError> {
        self.swap_manager.resume_all(&self.executor).await
    }

    /// Register an event listener. Returns a unique ID for removal.
    pub async fn add_event_listener(&self, listener: Box<dyn BoltzEventListener>) -> String {
        self.event_emitter.add_listener(listener).await
    }

    /// Remove a previously registered event listener.
    pub async fn remove_event_listener(&self, id: &str) -> bool {
        self.event_emitter.remove_listener(id).await
    }

    /// Get a swap by its internal ID.
    pub async fn get_swap(&self, swap_id: &str) -> Result<Option<BoltzSwap>, BoltzError> {
        self.executor.store.get_swap(swap_id).await
    }

    /// Shut down the swap manager and close the WebSocket connection.
    pub async fn shutdown(&self) {
        self.swap_manager.shutdown().await;
        self.ws_subscriber.close().await;
    }

    /// Get a quote for converting sats to USDT.
    /// Pure quote — no side effects, no swap created.
    pub async fn prepare_reverse_swap(
        &self,
        destination: &str,
        chain: Chain,
        usdt_amount: u64,
    ) -> Result<PreparedSwap, BoltzError> {
        self.executor.prepare(destination, chain, usdt_amount).await
    }

    /// Get a quote starting from input sats (computes expected USDT output).
    /// Pure quote — no side effects, no swap created.
    pub async fn prepare_reverse_swap_from_sats(
        &self,
        destination: &str,
        chain: Chain,
        invoice_amount_sats: u64,
    ) -> Result<PreparedSwap, BoltzError> {
        self.executor
            .prepare_from_sats(destination, chain, invoice_amount_sats)
            .await
    }

    /// Create the swap on Boltz and begin background monitoring.
    /// Returns the hold invoice to pay.
    pub async fn create_reverse_swap(
        &self,
        prepared: &PreparedSwap,
    ) -> Result<CreatedSwap, BoltzError> {
        let created = self.executor.create(prepared).await?;
        self.swap_manager.track_swap(&created.swap_id).await;
        Ok(created)
    }

    /// Get supported destination chains.
    pub fn supported_chains(&self) -> Vec<Chain> {
        vec![
            Chain::Arbitrum,
            Chain::Berachain,
            Chain::Conflux,
            Chain::Corn,
            Chain::Ethereum,
            Chain::Flare,
            Chain::Hedera,
            Chain::HyperEvm,
            Chain::Ink,
            Chain::Mantle,
            Chain::MegaEth,
            Chain::Monad,
            Chain::Morph,
            Chain::Optimism,
            Chain::Plasma,
            Chain::Polygon,
            Chain::Rootstock,
            Chain::Sei,
            Chain::Stable,
            Chain::Unichain,
            Chain::XLayer,
        ]
    }

    /// Get current Boltz swap limits (min/max sats).
    pub async fn get_limits(&self) -> Result<SwapLimits, BoltzError> {
        self.executor.get_limits().await
    }

    /// Recover unclaimed swaps by scanning the blockchain.
    pub async fn recover(&self, destination_address: &str) -> Result<RecoveryResult, BoltzError> {
        self.executor.recover(destination_address).await
    }
}
