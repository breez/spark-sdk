use tokio::sync::RwLock;
use tokio_with_wasm::alias as tokio;
use web_time::{SystemTime, UNIX_EPOCH};

/// A cell that holds a value with a time-to-live (TTL) expiration.
///
/// Similar to `OnceCell`, but the cached value expires after a specified duration.
/// After expiration, `get()` returns `None` and a new value can be set.
pub(crate) struct ExpiringCell<T> {
    inner: RwLock<Option<(T, u128)>>, // (value, expiration_ms)
}

impl<T> ExpiringCell<T> {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }
}

impl<T: Clone> ExpiringCell<T> {
    /// Returns the cached value if it exists and hasn't expired.
    pub async fn get(&self) -> Option<T> {
        let guard = self.inner.read().await;
        let (value, expiration) = guard.as_ref()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis();
        if now < *expiration {
            Some(value.clone())
        } else {
            None
        }
    }

    /// Sets a new value with the specified TTL in milliseconds.
    pub async fn set(&self, value: T, ttl_ms: u128) {
        let expiration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().saturating_add(ttl_ms))
            .unwrap_or(0);
        *self.inner.write().await = Some((value, expiration));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_with_wasm::alias as tokio;

    use super::ExpiringCell;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[macros::async_test_all]
    async fn test_expiring_cell_set_get() {
        let cell: ExpiringCell<String> = ExpiringCell::new();
        let value = "test_value".to_string();

        cell.set(value.clone(), 1000).await;

        let cached_value = cell.get().await;
        assert_eq!(cached_value, Some(value));
    }

    #[macros::async_test_all]
    async fn test_expiring_cell_expiration() {
        let cell: ExpiringCell<String> = ExpiringCell::new();
        let value = "test_value".to_string();

        cell.set(value, 10).await;

        tokio::time::sleep(Duration::from_millis(20)).await;

        let cached_value = cell.get().await;
        assert_eq!(cached_value, None);
    }

    #[macros::async_test_all]
    async fn test_expiring_cell_empty() {
        let cell: ExpiringCell<String> = ExpiringCell::new();

        let cached_value = cell.get().await;
        assert_eq!(cached_value, None);
    }

    #[macros::async_test_all]
    async fn test_expiring_cell_update() {
        let cell: ExpiringCell<String> = ExpiringCell::new();

        cell.set("value1".to_string(), 1000).await;
        cell.set("value2".to_string(), 1000).await;

        let cached_value = cell.get().await;
        assert_eq!(cached_value, Some("value2".to_string()));
    }
}
