//! Adapter exposing the SDK [`Storage`] as a [`boltz_client::BoltzStorage`].
//!
//! Canonical Boltz swap state lives in a dedicated cache KV namespace
//! rather than on payment metadata rows. The adapter is payment-row
//! agnostic: it can run at prepare time, before any [`crate::Payment`]
//! has been written.

use std::sync::Arc;

use boltz_client::{BoltzError, BoltzStorage, models::BoltzSwap};
use platform_utils::tokio::sync::Mutex;

use crate::Storage;

/// Cache KV key for a serialized [`BoltzSwap`] row.
fn swap_row_key(swap_id: &str) -> String {
    format!("boltz_swap_row_{swap_id}")
}

/// Cache KV key for the JSON array of currently-non-terminal swap ids.
const ACTIVE_SWAP_IDS_KEY: &str = "boltz_active_swap_ids";

/// Cache KV key for the per-instance claim-key counter.
fn key_index_cache_key(instance_id: &str) -> String {
    format!("boltz_key_index_{instance_id}")
}

/// Bridge between breez-sdk [`Storage`] and [`boltz_client::BoltzStorage`].
///
/// `instance_id` namespaces the local claim-key counter so two concurrent
/// SDK instances on the same wallet cannot collide on the same HD index. It
/// is also the forward-compat anchor for v2 (submarine swaps), which will
/// introduce a `BoltzInstance` rtsync record type and publish the existing
/// local seed retroactively for cross-device recovery. Nothing outside this
/// crate references the field in v1.
pub(crate) struct BoltzStorageAdapter {
    storage: Arc<dyn Storage>,
    instance_id: String,
    key_index_mutex: Arc<Mutex<()>>,
    /// Serializes read-modify-write of `boltz_active_swap_ids` across
    /// concurrent `insert_swap`/`update_swap` calls. Without it, two
    /// concurrent swap transitions can race on the load → mutate → save
    /// cycle and lose one another's update.
    active_ids_mutex: Arc<Mutex<()>>,
}

impl BoltzStorageAdapter {
    pub(crate) fn new(storage: Arc<dyn Storage>, instance_id: String) -> Self {
        Self {
            storage,
            instance_id,
            key_index_mutex: Arc::new(Mutex::new(())),
            active_ids_mutex: Arc::new(Mutex::new(())),
        }
    }

    async fn load_active_ids(&self) -> Result<Vec<String>, BoltzError> {
        let raw = self
            .storage
            .get_cached_item(ACTIVE_SWAP_IDS_KEY.to_string())
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to read active swap ids: {e}")))?;
        match raw {
            Some(json) => serde_json::from_str::<Vec<String>>(&json).map_err(|e| {
                BoltzError::Store(format!("Failed to deserialize active swap ids: {e}"))
            }),
            None => Ok(Vec::new()),
        }
    }

    async fn save_active_ids(&self, ids: &[String]) -> Result<(), BoltzError> {
        let json = serde_json::to_string(ids)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize active swap ids: {e}")))?;
        self.storage
            .set_cached_item(ACTIVE_SWAP_IDS_KEY.to_string(), json)
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to write active swap ids: {e}")))
    }
}

#[macros::async_trait]
impl BoltzStorage for BoltzStorageAdapter {
    async fn insert_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError> {
        let serialized = serde_json::to_string(swap)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize swap: {e}")))?;
        self.storage
            .set_cached_item(swap_row_key(&swap.id), serialized)
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to persist swap row: {e}")))?;

        if !swap.status.is_terminal() {
            let _guard = self.active_ids_mutex.lock().await;
            let mut ids = self.load_active_ids().await?;
            if !ids.contains(&swap.id) {
                ids.push(swap.id.clone());
                self.save_active_ids(&ids).await?;
            }
        }
        Ok(())
    }

    async fn update_swap(&self, swap: &BoltzSwap) -> Result<(), BoltzError> {
        let serialized = serde_json::to_string(swap)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize swap: {e}")))?;
        self.storage
            .set_cached_item(swap_row_key(&swap.id), serialized)
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to update swap row: {e}")))?;

        let _guard = self.active_ids_mutex.lock().await;
        let mut ids = self.load_active_ids().await?;
        let contained = ids.contains(&swap.id);
        if swap.status.is_terminal() {
            if contained {
                ids.retain(|id| id != &swap.id);
                self.save_active_ids(&ids).await?;
            }
        } else if !contained {
            ids.push(swap.id.clone());
            self.save_active_ids(&ids).await?;
        }
        Ok(())
    }

    async fn get_swap(&self, id: &str) -> Result<Option<BoltzSwap>, BoltzError> {
        let raw = self
            .storage
            .get_cached_item(swap_row_key(id))
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to read swap row: {e}")))?;
        match raw {
            Some(json) => serde_json::from_str::<BoltzSwap>(&json)
                .map(Some)
                .map_err(|e| BoltzError::Store(format!("Failed to deserialize swap row: {e}"))),
            None => Ok(None),
        }
    }

    async fn list_active_swaps(&self) -> Result<Vec<BoltzSwap>, BoltzError> {
        let ids = self.load_active_ids().await?;
        let mut swaps = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(swap) = self.get_swap(&id).await? {
                swaps.push(swap);
            }
        }
        Ok(swaps)
    }

    async fn increment_key_index(&self) -> Result<u32, BoltzError> {
        let _guard = self.key_index_mutex.lock().await;
        let key = key_index_cache_key(&self.instance_id);
        let current = self
            .storage
            .get_cached_item(key.clone())
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to read key index: {e}")))?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let next = current
            .checked_add(1)
            .ok_or_else(|| BoltzError::Store("Key index overflow".to_string()))?;
        self.storage
            .set_cached_item(key, next.to_string())
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to persist key index: {e}")))?;
        Ok(current)
    }

    async fn set_key_index_if_higher(&self, value: u32) -> Result<(), BoltzError> {
        let _guard = self.key_index_mutex.lock().await;
        let key = key_index_cache_key(&self.instance_id);
        let current = self
            .storage
            .get_cached_item(key.clone())
            .await
            .map_err(|e| BoltzError::Store(format!("Failed to read key index: {e}")))?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        if value > current {
            self.storage
                .set_cached_item(key, value.to_string())
                .await
                .map_err(|e| BoltzError::Store(format!("Failed to persist key index: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use boltz_client::models::{BoltzSwap, BoltzSwapStatus, Chain};

    use super::*;
    use crate::persist::sqlite::SqliteStorage;

    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn make_swap(id: &str, status: BoltzSwapStatus) -> BoltzSwap {
        BoltzSwap {
            id: id.to_string(),
            status,
            claim_key_index: 0,
            chain_id: 42161,
            claim_address: "0xclaim".to_string(),
            destination_address: "0xdest".to_string(),
            destination_chain: Chain::Arbitrum,
            refund_address: "0xrefund".to_string(),
            erc20swap_address: "0xswap".to_string(),
            router_address: "0xrouter".to_string(),
            invoice: "lnbc1000n".to_string(),
            invoice_amount_sats: 100_000,
            onchain_amount: 99_500,
            expected_usdt_amount: 71_000_000,
            slippage_bps: 100,
            timeout_block_height: 123_456,
            lockup_tx_id: None,
            claim_tx_hash: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    fn make_adapter() -> BoltzStorageAdapter {
        let dir = create_temp_dir("boltz_storage_adapter");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());
        BoltzStorageAdapter::new(storage, "instance-test".to_string())
    }

    #[tokio::test]
    async fn insert_get_list_active_roundtrip() {
        let adapter = make_adapter();
        let swap = make_swap("s1", BoltzSwapStatus::Created);
        adapter.insert_swap(&swap).await.unwrap();

        let fetched = adapter.get_swap("s1").await.unwrap().unwrap();
        assert_eq!(fetched.id, "s1");
        assert_eq!(fetched.status, BoltzSwapStatus::Created);

        let active = adapter.list_active_swaps().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "s1");
    }

    #[tokio::test]
    async fn concurrent_inserts_do_not_drop_active_ids() {
        // Spawn many concurrent `insert_swap` calls. Without the active_ids
        // mutex the load → mutate → save cycle would race and drop entries.
        const N: usize = 32;

        let dir = create_temp_dir("boltz_storage_adapter_concurrent");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new(&dir).unwrap());
        let adapter = Arc::new(BoltzStorageAdapter::new(
            storage,
            "concurrent-test".to_string(),
        ));

        let mut handles = Vec::with_capacity(N);
        for i in 0..N {
            let adapter = Arc::clone(&adapter);
            handles.push(tokio::spawn(async move {
                let swap = make_swap(&format!("race_{i}"), BoltzSwapStatus::Created);
                adapter.insert_swap(&swap).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let active = adapter.list_active_swaps().await.unwrap();
        assert_eq!(
            active.len(),
            N,
            "all concurrent insert_swap calls must land in active_ids"
        );
    }

    #[tokio::test]
    async fn update_to_terminal_removes_from_active() {
        let adapter = make_adapter();
        adapter
            .insert_swap(&make_swap("s1", BoltzSwapStatus::Created))
            .await
            .unwrap();
        adapter
            .insert_swap(&make_swap("s2", BoltzSwapStatus::Created))
            .await
            .unwrap();

        let terminal = make_swap("s1", BoltzSwapStatus::Completed);
        adapter.update_swap(&terminal).await.unwrap();

        let active = adapter.list_active_swaps().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "s2");

        // terminal swap is still retrievable by id
        let fetched = adapter.get_swap("s1").await.unwrap().unwrap();
        assert_eq!(fetched.status, BoltzSwapStatus::Completed);
    }

    #[tokio::test]
    async fn increment_key_index_persists_and_is_monotonic() {
        let adapter = make_adapter();

        let first = adapter.increment_key_index().await.unwrap();
        let second = adapter.increment_key_index().await.unwrap();
        let third = adapter.increment_key_index().await.unwrap();

        assert_eq!(first, 0);
        assert_eq!(second, 1);
        assert_eq!(third, 2);
    }

    #[tokio::test]
    async fn set_key_index_if_higher_only_grows() {
        let adapter = make_adapter();

        adapter.increment_key_index().await.unwrap(); // 0 -> 1
        adapter.increment_key_index().await.unwrap(); // 1 -> 2
        adapter.set_key_index_if_higher(10).await.unwrap();

        let next = adapter.increment_key_index().await.unwrap();
        assert_eq!(next, 10);

        // lower values are ignored
        adapter.set_key_index_if_higher(5).await.unwrap();
        let after = adapter.increment_key_index().await.unwrap();
        assert_eq!(after, 11);
    }
}
