pub mod api;
pub mod config;
pub mod error;
pub mod evm;
pub mod keys;
pub mod models;
pub mod recover;
pub mod store;
pub mod swap;

use std::sync::Arc;

use platform_utils::DefaultHttpClient;

pub use config::*;
pub use error::BoltzError;
pub use keys::EvmKeyManager;
pub use models::*;
pub use store::{BoltzStorage, MemoryBoltzStorage};

use api::BoltzApiClient;
use api::ws::SwapStatusSubscriber;
use evm::alchemy::AlchemyGasClient;
use evm::oft::OftDeployments;
use evm::provider::EvmProvider;
use evm::signing::EvmSigner;
use swap::reverse::ReverseSwapExecutor;

/// Top-level Boltz service facade.
///
/// Three-step flow:
/// - `prepare_reverse_swap` — pure quote, no side effects
/// - `create_reverse_swap` — commit to swap, get invoice
/// - `complete_reverse_swap` — monitor + claim (blocks until done)
pub struct BoltzService {
    executor: ReverseSwapExecutor,
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

        let api_client = BoltzApiClient::new(&config, Box::new(DefaultHttpClient::new(None)));
        let ws_subscriber = SwapStatusSubscriber::connect(&config.ws_url()).await?;

        let alchemy_client = AlchemyGasClient::new(
            &config.alchemy_config,
            Box::new(DefaultHttpClient::new(None)),
            gas_signer,
        );

        let evm_provider = EvmProvider::new(
            config.arbitrum_rpc_url.clone(),
            Box::new(DefaultHttpClient::new(None)),
        );

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

        let executor = ReverseSwapExecutor::new(
            api_client,
            ws_subscriber,
            key_manager,
            alchemy_client,
            evm_provider,
            oft_deployments,
            store,
            config,
            erc20swap_address,
        );

        Ok(Self { executor })
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

    /// Create the swap on Boltz. Returns the hold invoice to pay.
    /// Caller must pay the invoice via Lightning.
    pub async fn create_reverse_swap(
        &self,
        prepared: &PreparedSwap,
    ) -> Result<CreatedSwap, BoltzError> {
        self.executor.create(prepared).await
    }

    /// After the invoice is paid, monitor and complete the swap.
    /// Blocks until USDT is delivered or swap fails.
    pub async fn complete_reverse_swap(&self, swap_id: &str) -> Result<CompletedSwap, BoltzError> {
        self.executor.complete(swap_id).await
    }

    /// Resume all active (non-final) swaps from storage.
    /// Call on startup to recover interrupted swaps.
    pub async fn resume(&self) -> Result<Vec<String>, BoltzError> {
        self.executor.resume_active_swaps().await
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
    /// Uses EVM contract log scanning to find Lockup events matching
    /// this wallet's derived keys, then claims any still-locked swaps.
    pub async fn recover(&self, destination_address: &str) -> Result<RecoveryResult, BoltzError> {
        self.executor.recover(destination_address).await
    }
}
