use crate::error::BoltzError;
use crate::models::BoltzSwap;

/// Persistence interface for Boltz swap state.
///
/// The boltz crate defines the trait; the caller provides the implementation.
/// For testing, use `MemoryBoltzStorage`.
#[macros::async_trait]
pub trait BoltzStorage: Send + Sync {
    async fn insert_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError>;
    async fn update_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError>;
    async fn get_swap(&self, id: &str) -> Result<Option<BoltzSwap>, BoltzError>;
    /// Return all swaps with non-terminal status.
    async fn list_active_swaps(&self) -> Result<Vec<BoltzSwap>, BoltzError>;
    /// Atomically reserve the next key index and return it.
    async fn increment_key_index(&self, chain_id: u64) -> Result<u32, BoltzError>;
    /// Set the key index to `value` if it is greater than the current index.
    /// Used by recovery to fast-forward past already-used indices.
    async fn set_key_index_if_higher(&self, chain_id: u64, value: u32) -> Result<(), BoltzError>;
}

/// In-memory store for testing.
#[derive(Default)]
pub struct MemoryBoltzStorage {
    swaps: tokio::sync::Mutex<std::collections::HashMap<String, BoltzSwap>>,
    key_indices: tokio::sync::Mutex<std::collections::HashMap<u64, u32>>,
}

impl MemoryBoltzStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[macros::async_trait]
impl BoltzStorage for MemoryBoltzStorage {
    async fn insert_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError> {
        self.swaps
            .lock()
            .await
            .insert(swap.id.clone(), swap.clone());
        Ok(())
    }

    async fn update_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError> {
        let mut swaps = self.swaps.lock().await;
        if swaps.contains_key(&swap.id) {
            swaps.insert(swap.id.clone(), swap.clone());
            Ok(())
        } else {
            Err(BoltzError::Store(format!("Swap not found: {}", swap.id)))
        }
    }

    async fn get_swap(&self, id: &str) -> Result<Option<BoltzSwap>, BoltzError> {
        Ok(self.swaps.lock().await.get(id).cloned())
    }

    async fn list_active_swaps(&self) -> Result<Vec<BoltzSwap>, BoltzError> {
        Ok(self
            .swaps
            .lock()
            .await
            .values()
            .filter(|s| !s.status.is_terminal())
            .cloned()
            .collect())
    }

    async fn increment_key_index(&self, chain_id: u64) -> Result<u32, BoltzError> {
        let mut indices = self.key_indices.lock().await;
        let idx = indices.entry(chain_id).or_insert(0);
        let current = *idx;
        *idx = current
            .checked_add(1)
            .ok_or_else(|| BoltzError::Store("Key index overflow".to_string()))?;
        Ok(current)
    }

    async fn set_key_index_if_higher(&self, chain_id: u64, value: u32) -> Result<(), BoltzError> {
        let mut indices = self.key_indices.lock().await;
        let current = indices.entry(chain_id).or_insert(0);
        if value > *current {
            *current = value;
        }
        Ok(())
    }
}

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use super::*;
    use crate::models::{BoltzSwapStatus, Chain};

    fn test_swap(id: &str, status: BoltzSwapStatus) -> BoltzSwap {
        BoltzSwap {
            id: id.to_string(),
            status,
            claim_key_index: 0,
            chain_id: 42161,
            claim_address: "0xabc".to_string(),
            destination_address: "0xdef".to_string(),
            destination_chain: Chain::Arbitrum,
            refund_address: "0x123".to_string(),
            erc20swap_address: "0xswap".to_string(),
            router_address: "0xrouter".to_string(),
            invoice: "lnbc...".to_string(),
            invoice_amount_sats: 100_000,
            onchain_amount: 99_500,
            expected_usdt_amount: 71_000_000,
            timeout_block_height: 123_456,
            lockup_tx_id: None,
            claim_tx_hash: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    #[tokio::test]
    async fn test_insert_and_get() {
        let store = MemoryBoltzStorage::new();
        let swap = test_swap("1", BoltzSwapStatus::Created);
        store.insert_swap(&swap).await.unwrap();

        let retrieved = store.get_swap("1").await.unwrap().unwrap();
        assert_eq!(retrieved.id, "1");
    }

    #[tokio::test]
    async fn test_update_swap() {
        let store = MemoryBoltzStorage::new();
        let mut swap = test_swap("1", BoltzSwapStatus::Created);
        store.insert_swap(&swap).await.unwrap();

        swap.status = BoltzSwapStatus::TbtcLocked;
        store.update_swap(&swap).await.unwrap();

        let retrieved = store.get_swap("1").await.unwrap().unwrap();
        assert_eq!(retrieved.status, BoltzSwapStatus::TbtcLocked);
    }

    #[tokio::test]
    async fn test_update_nonexistent_fails() {
        let store = MemoryBoltzStorage::new();
        let swap = test_swap("1", BoltzSwapStatus::Created);
        assert!(store.update_swap(&swap).await.is_err());
    }

    #[tokio::test]
    async fn test_list_active_swaps() {
        let store = MemoryBoltzStorage::new();
        store
            .insert_swap(&test_swap("1", BoltzSwapStatus::Created))
            .await
            .unwrap();
        store
            .insert_swap(&test_swap("2", BoltzSwapStatus::Completed))
            .await
            .unwrap();
        store
            .insert_swap(&test_swap("3", BoltzSwapStatus::TbtcLocked))
            .await
            .unwrap();

        let active = store.list_active_swaps().await.unwrap();
        assert_eq!(active.len(), 2);
    }

    #[tokio::test]
    async fn test_key_index_management() {
        let store = MemoryBoltzStorage::new();

        let idx0 = store.increment_key_index(42161).await.unwrap();
        assert_eq!(idx0, 0);

        let idx1 = store.increment_key_index(42161).await.unwrap();
        assert_eq!(idx1, 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent_returns_none() {
        let store = MemoryBoltzStorage::new();
        assert!(store.get_swap("nonexistent").await.unwrap().is_none());
    }
}
