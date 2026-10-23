use thiserror::Error;

#[derive(Debug, Error, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum X402StoreError {
    #[error("Generic error: {0}")]
    Generic(String),
}

/// Records which x402 payments have been used, so a payment can't pay for
/// two requests. Every server instance charging for the same SDK must share
/// one store, and it must survive restarts: a lost record lets a paid
/// invoice be presented again.
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait X402ReplayStore: Send + Sync {
    /// Records `key` as used, atomically: of two concurrent calls with the
    /// same key, exactly one returns `true`. The record must be kept until
    /// `retain_until_secs`, a Unix time, and may be deleted after it.
    async fn consume(&self, key: String, retain_until_secs: u64) -> Result<bool, X402StoreError>;
}

/// Tests every [`X402ReplayStore`] implementation runs. `store` and `other`
/// must belong to different tenants.
#[cfg(any(test, feature = "test-utils"))]
pub mod x402_replay_store_tests {
    use super::X402ReplayStore;

    /// 2106, representable exactly as a JavaScript number.
    const FAR_FUTURE: u64 = u32::MAX as u64;

    pub async fn run_all(store: &dyn X402ReplayStore, other: &dyn X402ReplayStore) {
        consumes_each_key_once(store).await;
        tenants_are_independent(store, other).await;
        concurrent_consumes_have_one_winner(store).await;
        prunes_records_past_retention(store).await;
    }

    async fn consumes_each_key_once(store: &dyn X402ReplayStore) {
        assert!(
            store
                .consume("lnbtc:a:key".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
        assert!(
            !store
                .consume("lnbtc:a:key".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
        assert!(
            store
                .consume("lnbtc:b:key".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
    }

    async fn tenants_are_independent(store: &dyn X402ReplayStore, other: &dyn X402ReplayStore) {
        assert!(
            store
                .consume("lnbtc:a:shared".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
        assert!(
            other
                .consume("lnbtc:a:shared".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
    }

    async fn concurrent_consumes_have_one_winner(store: &dyn X402ReplayStore) {
        let attempts = (0..8).map(|_| store.consume("lnbtc:a:race".to_string(), FAR_FUTURE));
        let results = futures::future::join_all(attempts).await;
        let winners = results.into_iter().filter(|r| *r.as_ref().unwrap()).count();
        assert_eq!(winners, 1);
    }

    async fn prunes_records_past_retention(store: &dyn X402ReplayStore) {
        // Retained until 1970, so the next consume prunes it and it can be used
        // again. By then its invoice is long expired, so this is harmless.
        assert!(store.consume("lnbtc:a:old".to_string(), 1).await.unwrap());
        assert!(
            store
                .consume("lnbtc:a:new".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
        assert!(
            store
                .consume("lnbtc:a:old".to_string(), FAR_FUTURE)
                .await
                .unwrap()
        );
    }
}
