use serde::{Deserialize, Serialize};

/// Persisted state for a single Boltz reverse swap.
///
/// Preimage and `preimage_hash` are NOT stored — they are deterministically
/// derived from `seed + claim_key_index + chain_id`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoltzSwap {
    /// Internal UUID.
    pub id: String,
    /// Boltz swap ID (from create response).
    pub boltz_id: String,
    pub status: BoltzSwapStatus,
    /// HD derivation index for the per-swap preimage key.
    pub claim_key_index: u32,
    /// EVM chain ID (42161 for Arbitrum).
    pub chain_id: u64,

    // Addresses
    /// Gas signer address (used as claimAddress with Boltz).
    pub claim_address: String,
    /// User's final USDT destination.
    pub destination_address: String,
    /// Target chain for delivery.
    pub destination_chain: Chain,
    /// Boltz's refund address (from swap response).
    pub refund_address: String,

    // Contract addresses (snapshot at creation time)
    pub erc20swap_address: String,
    pub router_address: String,

    // Invoice
    pub invoice: String,
    pub invoice_amount_sats: u64,

    // Amounts
    /// tBTC amount locked on-chain (sats, from swap response `onchainAmount`).
    pub onchain_amount: u64,
    /// Expected USDT output (6 decimals).
    pub expected_usdt_amount: u64,

    // Timing
    pub timeout_block_height: u64,

    // Results
    pub lockup_tx_id: Option<String>,
    pub claim_tx_hash: Option<String>,

    // Timestamps (unix seconds)
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BoltzSwapStatus {
    /// Swap created on Boltz, invoice ready to pay.
    Created,
    /// Hold invoice paid, waiting for Boltz to lock tBTC.
    InvoicePaid,
    /// tBTC locked on Arbitrum, ready to claim.
    TbtcLocked,
    /// Claim tx submitted, waiting for confirmation.
    Claiming,
    /// USDT delivered to destination.
    Completed,
    /// Swap failed.
    Failed { reason: String },
    /// Swap expired (Boltz timeout reached).
    Expired,
}

impl BoltzSwapStatus {
    /// Whether this status is terminal (no further transitions expected).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed { .. } | Self::Expired)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Chain {
    Arbitrum,
    Berachain,
    Conflux,
    Corn,
    Ethereum,
    Flare,
    Hedera,
    HyperEvm,
    Ink,
    Mantle,
    MegaEth,
    Monad,
    Morph,
    Optimism,
    Plasma,
    Polygon,
    Rootstock,
    Sei,
    Stable,
    Unichain,
    XLayer,
}

impl Chain {
    /// EVM chain ID for this chain.
    pub fn evm_chain_id(&self) -> u64 {
        match self {
            Self::Arbitrum => 42161,
            Self::Berachain => 80094,
            Self::Conflux => 1030,
            Self::Corn => 21_000_000,
            Self::Ethereum => 1,
            Self::Flare => 14,
            Self::Hedera => 295,
            Self::HyperEvm => 999,
            Self::Ink => 57073,
            Self::Mantle => 5000,
            Self::MegaEth => 4326,
            Self::Monad => 143,
            Self::Morph => 2818,
            Self::Optimism => 10,
            Self::Plasma => 9745,
            Self::Polygon => 137,
            Self::Rootstock => 30,
            Self::Sei => 1329,
            Self::Stable => 988,
            Self::Unichain => 130,
            Self::XLayer => 196,
        }
    }

    /// Whether this is the source chain (Arbitrum) where claims happen on-chain.
    /// Non-Arbitrum destinations require OFT cross-chain bridging.
    pub fn is_source_chain(&self) -> bool {
        *self == Self::Arbitrum
    }
}

/// Quote result returned to caller before committing to a swap.
#[derive(Clone, Debug)]
pub struct PreparedSwap {
    pub destination_address: String,
    pub destination_chain: Chain,
    /// Requested USDT output (6 decimals).
    pub usdt_amount: u64,
    /// Total sats to pay (includes all fees).
    pub invoice_amount_sats: u64,
    /// Boltz service fee in sats.
    pub boltz_fee_sats: u64,
    /// tBTC amount after Boltz fee (sats).
    pub estimated_onchain_amount: u64,
    /// Estimated USDT after DEX swap + slippage.
    pub estimated_usdt_output: u64,
    pub slippage_bps: u32,
    /// Pins fee/rate snapshot for `POST /swap/reverse`.
    pub pair_hash: String,
    /// Quote expiry (unix timestamp seconds).
    pub expires_at: u64,
}

/// Result of creating a swap on Boltz.
#[derive(Clone, Debug)]
pub struct CreatedSwap {
    /// Internal swap ID.
    pub swap_id: String,
    /// Boltz swap ID.
    pub boltz_id: String,
    /// Hold invoice to pay.
    pub invoice: String,
    pub invoice_amount_sats: u64,
    pub timeout_block_height: u64,
}

/// Result of a successfully completed swap.
#[derive(Clone, Debug)]
pub struct CompletedSwap {
    pub swap_id: String,
    pub claim_tx_hash: String,
    /// Actual USDT amount delivered (6 decimals).
    pub usdt_delivered: u64,
    pub destination_address: String,
    pub destination_chain: Chain,
}

/// Min/max swap limits from the Boltz pairs endpoint.
#[derive(Clone, Debug)]
pub struct SwapLimits {
    pub min_sats: u64,
    pub max_sats: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swap_status_terminal() {
        assert!(!BoltzSwapStatus::Created.is_terminal());
        assert!(!BoltzSwapStatus::InvoicePaid.is_terminal());
        assert!(!BoltzSwapStatus::TbtcLocked.is_terminal());
        assert!(!BoltzSwapStatus::Claiming.is_terminal());
        assert!(BoltzSwapStatus::Completed.is_terminal());
        assert!(BoltzSwapStatus::Expired.is_terminal());
        assert!(
            BoltzSwapStatus::Failed {
                reason: "test".to_string()
            }
            .is_terminal()
        );
    }

    #[test]
    fn test_boltz_swap_serialization() {
        let swap = BoltzSwap {
            id: "uuid-1".to_string(),
            boltz_id: "boltz-1".to_string(),
            status: BoltzSwapStatus::Created,
            claim_key_index: 0,
            chain_id: 42161,
            claim_address: "0xabc".to_string(),
            destination_address: "0xdef".to_string(),
            destination_chain: Chain::Arbitrum,
            refund_address: "0x123".to_string(),
            erc20swap_address: "0xswap".to_string(),
            router_address: "0xrouter".to_string(),
            invoice: "lnbc1000n1...".to_string(),
            invoice_amount_sats: 100_000,
            onchain_amount: 99_500,
            expected_usdt_amount: 71_000_000,
            timeout_block_height: 123_456,
            lockup_tx_id: None,
            claim_tx_hash: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        };

        let json = serde_json::to_string(&swap).unwrap();
        let deserialized: BoltzSwap = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.boltz_id, "boltz-1");
        assert_eq!(deserialized.status, BoltzSwapStatus::Created);
        assert_eq!(deserialized.chain_id, 42161);
    }

    #[test]
    fn test_chain_equality() {
        assert_eq!(Chain::Arbitrum, Chain::Arbitrum);
        assert_ne!(Chain::Arbitrum, Chain::Ethereum);
    }

    #[test]
    fn test_evm_chain_id() {
        assert_eq!(Chain::Arbitrum.evm_chain_id(), 42161);
        assert_eq!(Chain::Ethereum.evm_chain_id(), 1);
        assert_eq!(Chain::Optimism.evm_chain_id(), 10);
        assert_eq!(Chain::Polygon.evm_chain_id(), 137);
    }

    #[test]
    fn test_is_source_chain() {
        assert!(Chain::Arbitrum.is_source_chain());
        assert!(!Chain::Ethereum.is_source_chain());
        assert!(!Chain::Optimism.is_source_chain());
    }
}
