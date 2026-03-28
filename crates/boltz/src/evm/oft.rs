//! OFT deployment registry — fetches `LayerZero` endpoint IDs and OFT contract
//! addresses from the USDT0 API at runtime, matching the Boltz web app.

use std::collections::HashMap;

use platform_utils::http::HttpClient;
use serde::Deserialize;

use crate::error::BoltzError;

/// API endpoint for OFT deployments (same as web app's `oftDeploymentsEndpoint`).
const OFT_DEPLOYMENTS_URL: &str = "https://docs.usdt0.to/api/deployments";

/// Default OFT token name to look up (matches web app's `defaultOftName`).
const DEFAULT_OFT_NAME: &str = "usdt0";

/// Resolved OFT info for a single chain.
#[derive(Clone, Debug)]
pub struct OftChainInfo {
    /// `LayerZero` endpoint ID for this chain.
    pub lz_eid: u32,
    /// OFT contract address on this chain (hex with 0x prefix).
    pub oft_address: String,
}

/// Cached OFT deployment data, keyed by EVM chain ID.
#[derive(Clone, Debug)]
pub struct OftDeployments {
    chains: HashMap<u64, OftChainInfo>,
}

impl OftDeployments {
    /// Fetch OFT deployments from the USDT0 API.
    pub async fn fetch(http_client: &dyn HttpClient) -> Result<Self, BoltzError> {
        let response = http_client
            .get(OFT_DEPLOYMENTS_URL.to_string(), None)
            .await?;

        if !response.is_success() {
            return Err(BoltzError::Api {
                reason: format!("Failed to fetch OFT deployments: HTTP {}", response.status),
                code: None,
            });
        }

        let registry: OftRegistry =
            serde_json::from_str(&response.body).map_err(|e| BoltzError::Api {
                reason: format!("Failed to parse OFT deployments: {e}"),
                code: None,
            })?;

        let token_config = registry
            .0
            .get(DEFAULT_OFT_NAME)
            .ok_or_else(|| BoltzError::Api {
                reason: format!("OFT token '{DEFAULT_OFT_NAME}' not found in deployments"),
                code: None,
            })?;

        let mut chains = HashMap::new();
        for chain in &token_config.native {
            let Some(chain_id) = chain.chain_id else {
                continue;
            };
            let Some(ref lz_eid_str) = chain.lz_eid else {
                continue;
            };
            let lz_eid: u32 = lz_eid_str.parse().map_err(|_| BoltzError::Api {
                reason: format!("Invalid lzEid '{lz_eid_str}' for chain {chain_id}"),
                code: None,
            })?;

            // Find OFT or OFT Adapter contract (matching web app's getOftContract)
            let oft_contract = chain
                .contracts
                .iter()
                .find(|c| c.name == "OFT")
                .or_else(|| chain.contracts.iter().find(|c| c.name == "OFT Adapter"));

            if let Some(contract) = oft_contract {
                chains.insert(
                    u64::from(chain_id),
                    OftChainInfo {
                        lz_eid,
                        oft_address: contract.address.clone(),
                    },
                );
            }
        }

        Ok(Self { chains })
    }

    /// Look up OFT info for a chain by EVM chain ID.
    pub fn get(&self, evm_chain_id: u64) -> Option<&OftChainInfo> {
        self.chains.get(&evm_chain_id)
    }

    /// Get the OFT contract address for the source chain (Arbitrum).
    pub fn source_oft_address(&self, source_chain_id: u64) -> Option<&str> {
        self.chains
            .get(&source_chain_id)
            .map(|info| info.oft_address.as_str())
    }
}

// ─── API response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct OftRegistry(HashMap<String, OftTokenConfig>);

#[derive(Deserialize)]
struct OftTokenConfig {
    native: Vec<OftApiChain>,
    // legacyMesh not used, matching web app's TODO
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OftApiChain {
    #[allow(dead_code)]
    name: String,
    chain_id: Option<u32>,
    lz_eid: Option<String>,
    contracts: Vec<OftApiContract>,
}

#[derive(Deserialize)]
struct OftApiContract {
    name: String,
    address: String,
    #[allow(dead_code)]
    explorer: String,
}
