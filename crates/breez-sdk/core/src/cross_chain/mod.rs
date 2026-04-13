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

use breez_sdk_common::input::{CrossChainAddressFamily, CrossChainRoutePair};
use serde::{Deserialize, Serialize};

use crate::error::SdkError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum CrossChainProvider {
    Orchestra,
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

    pub fn iter(&self) -> impl Iterator<Item = (&CrossChainProvider, &Arc<dyn CrossChainService>)> {
        self.0.iter()
    }
}

/// Data stashed on the prepared send payment so the provider can resume
/// the send stage without re-quoting.
#[derive(Debug, Clone)]
pub(crate) struct CrossChainPrepared {
    pub quote_id: String,
    pub deposit_address: String,
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
    /// Returns the available `{chain, asset}` route pairs for a given address
    /// family, optionally filtered by asset.
    async fn get_routes(
        &self,
        family: CrossChainAddressFamily,
        asset: Option<&str>,
    ) -> Result<Vec<CrossChainRoutePair>, SdkError>;

    /// Fetch a quote for a cross-chain send.
    async fn prepare(
        &self,
        recipient_address: &str,
        dest_chain: &str,
        dest_asset: &str,
        amount: u128,
        source_token_identifier: Option<String>,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainPrepared, SdkError>;

    /// Execute the send: transfer funds to the deposit address, submit to
    /// the provider, persist metadata, and trigger monitoring.
    async fn send(&self, prepared: &CrossChainPrepared) -> Result<CrossChainSendResult, SdkError>;
}
