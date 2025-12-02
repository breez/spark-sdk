use std::collections::HashMap;

use tokio::sync::Mutex;
use tokio_with_wasm::alias as tokio;
use web_time::{SystemTime, UNIX_EPOCH};

use crate::FlashnetError;

struct CacheItem {
    data: String,
    expiration: u128,
}

pub struct CacheStore {
    cache: Mutex<HashMap<String, CacheItem>>,
}

impl Default for CacheStore {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl CacheStore {
    pub async fn get<D>(&self, key: &str) -> Result<Option<D>, FlashnetError>
    where
        D: serde::de::DeserializeOwned,
    {
        let cache = self.cache.lock().await;
        let Some(item) = cache.get(key) else {
            return Ok(None);
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| FlashnetError::Generic(format!("System time error: {e}")))?
            .as_millis();
        if item.expiration < now {
            return Ok(None);
        }
        let res = serde_json::from_str(&item.data).map_err(|e| {
            FlashnetError::Generic(format!("Failed to deserialize cache item: {e}"))
        })?;
        Ok(Some(res))
    }

    pub async fn set<S>(&self, key: &str, data: &S, ttl_ms: u128) -> Result<(), FlashnetError>
    where
        S: serde::Serialize,
    {
        let mut cache = self.cache.lock().await;
        let data = serde_json::to_string(data)
            .map_err(|e| FlashnetError::Generic(format!("Failed to serialize cache item: {e}")))?;
        let expiration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| FlashnetError::Generic(format!("System time error: {e}")))?
            .as_millis()
            .saturating_add(ttl_ms);
        cache.insert(key.to_string(), CacheItem { data, expiration });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_with_wasm::alias as tokio;

    use super::CacheStore;

    #[macros::async_test_all]
    async fn test_cache_store_set_get() {
        let cache_store = CacheStore::default();
        let key = "test_key";
        let value = "test_value";

        // Set the value in the cache
        cache_store.set(key, &value, 1000).await.unwrap();

        // Get the value from the cache
        let cached_value: Option<String> = cache_store.get(key).await.unwrap();
        assert_eq!(cached_value, Some(value.to_string()));
    }

    #[macros::async_test_all]
    async fn test_cache_store_expiration() {
        let cache_store = CacheStore::default();
        let key = "test_key_expire";
        let value = "test_value_expire";
        // Set the value in the cache with a short TTL
        cache_store.set(key, &value, 10).await.unwrap();
        // Wait for the TTL to expire
        tokio::time::sleep(Duration::from_millis(20)).await;
        // Try to get the value from the cache
        let cached_value: Option<String> = cache_store.get(key).await.unwrap();
        assert_eq!(cached_value, None);
    }

    #[macros::async_test_all]
    async fn test_cache_store_nonexistent_key() {
        let cache_store = CacheStore::default();
        let key = "nonexistent_key";
        // Try to get a value for a nonexistent key
        let cached_value: Option<String> = cache_store.get(key).await.unwrap();
        assert_eq!(cached_value, None);
    }

    #[macros::async_test_all]
    async fn test_cache_store_update() {
        let cache_store = CacheStore::default();
        let key = "update_key";
        let value1 = "value1";
        let value2 = "value2";
        // Set the first value in the cache
        cache_store.set(key, &value1, 1000).await.unwrap();
        // Update the value in the cache
        cache_store.set(key, &value2, 1000).await.unwrap();
        // Get the updated value from the cache
        let cached_value: Option<String> = cache_store.get(key).await.unwrap();
        assert_eq!(cached_value, Some(value2.to_string()));
    }
}
