//! Cross-chain payment providers.
//!
//! The [`CrossChainService`] trait abstracts route discovery, quoting, and
//! sending. Each provider module (e.g. `orchestra`) implements it.
//! Currently only Orchestra (Flashnet) is implemented; future providers
//! (e.g. Boltz) will be sibling modules implementing the same trait.

mod orchestra;

pub(crate) use orchestra::OrchestraService;

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::SdkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CrossChainProvider {
    Orchestra,
}

/// A single route available for cross-chain sends, tagged with the provider
/// that offers it. Returned by `get_cross_chain_routes()`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CrossChainRoutePair {
    /// Which provider offers this route.
    pub provider: CrossChainProvider,
    /// Destination blockchain (e.g. `"base"`, `"solana"`, `"tron"`).
    pub chain: String,
    /// Destination asset symbol (e.g. `"USDC"`, `"USDT"`).
    pub asset: String,
    /// Token contract / mint address on the destination chain.
    pub contract_address: Option<String>,
    /// Decimal places for the destination asset.
    pub decimals: u8,
    /// Whether the route supports exact-out mode.
    pub exact_out_eligible: bool,
}

/// Registry of cross-chain providers keyed by [`CrossChainProvider`].
#[derive(Clone, Default)]
pub(crate) struct CrossChainProviders(HashMap<CrossChainProvider, Arc<dyn CrossChainService>>);

impl CrossChainProviders {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn insert(&mut self, key: CrossChainProvider, service: Arc<dyn CrossChainService>) {
        self.0.insert(key, service);
    }

    /// Look up a provider, returning a friendly error if missing.
    pub fn get(
        &self,
        provider: CrossChainProvider,
    ) -> Result<&Arc<dyn CrossChainService>, SdkError> {
        self.0.get(&provider).ok_or_else(|| {
            SdkError::Generic(format!("Cross-chain provider {provider:?} not available"))
        })
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<dyn CrossChainService>> {
        self.0.values()
    }
}

/// Data stashed on the prepared send payment so the provider can resume
/// the send stage without re-quoting.
#[derive(Debug, Clone)]
pub(crate) struct CrossChainPrepared {
    pub quote_id: String,
    /// Provider-specific deposit request. Orchestra uses a Spark deposit
    /// address; other providers (e.g. Boltz) may use a BOLT11 invoice.
    pub deposit_request: String,
    pub amount_in: u128,
    pub estimated_out: u128,
    pub fee_amount: u128,
    pub fee_bps: u32,
    pub expires_at: String,
    pub pair: CrossChainRoutePair,
    pub recipient_address: String,
    /// The `token_identifier` on the Spark source (e.g. USDB). `None` for BTC sats.
    pub token_identifier: Option<String>,
}

/// Result of a cross-chain send submission.
#[derive(Debug, Clone)]
pub(crate) struct CrossChainSendResult {
    pub order_id: String,
    /// The Spark payment ID used to store metadata. This is the ID to use
    /// when looking up the payment in storage.
    pub payment_id: String,
}

/// Abstraction over cross-chain bridge/swap providers.
///
/// Each implementation owns its own client, caching, and background monitoring.
/// The SDK dispatches to the provider via this trait.
#[macros::async_trait]
pub(crate) trait CrossChainService: Send + Sync {
    /// Returns the available route pairs for the given parsed address details.
    /// Providers filter by address family, contract address, chain ID, etc.
    async fn get_routes(
        &self,
        address_details: &crate::CrossChainAddressDetails,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError>;

    /// Fetch a quote for a cross-chain send.
    async fn prepare(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        source_token_identifier: Option<String>,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError>;

    /// Execute the send: transfer funds to the deposit address, submit to
    /// the provider, persist metadata, and trigger monitoring.
    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError>;
}
