use std::collections::HashMap;
use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use platform_utils::tokio;

/// Internal decorator that adds an in-memory cache in front of an inner
/// [`spark_wallet::SessionStore`]. Reads are served from the cache when
/// present and still valid; misses (or expired entries) fall through to the
/// inner store and the result is cached. Writes update both layers — inner
/// first (treating it as the source of truth) and the cache only on success.
///
/// Sits at the outermost layer of the SDK's session-store stack so the
/// auth providers' hot path is plaintext, in-process, and (typically)
/// allocation-free.
pub(crate) struct CachingSessionStore {
    inner: Arc<dyn spark_wallet::SessionStore>,
    cache: tokio::sync::Mutex<HashMap<PublicKey, spark_wallet::Session>>,
}

impl CachingSessionStore {
    pub(crate) fn new(inner: Arc<dyn spark_wallet::SessionStore>) -> Self {
        Self {
            inner,
            cache: tokio::sync::Mutex::new(HashMap::new()),
        }
    }
}

#[macros::async_trait]
impl spark_wallet::SessionStore for CachingSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
        if let Some(cached) = self.cache.lock().await.get(service_identity_key)
            && cached.is_valid()
        {
            return Ok(cached.clone());
        }
        let session = self.inner.get_session(service_identity_key).await?;
        self.cache
            .lock()
            .await
            .insert(*service_identity_key, session.clone());
        Ok(session)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: spark_wallet::Session,
    ) -> Result<(), spark_wallet::SessionStoreError> {
        self.inner
            .set_session(service_identity_key, session.clone())
            .await?;
        self.cache
            .lock()
            .await
            .insert(*service_identity_key, session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use macros::async_test_all;
    use platform_utils::time::{SystemTime, UNIX_EPOCH};
    use spark_wallet::SessionStore as _;

    use super::*;

    /// In-memory `spark_wallet::SessionStore` that counts how often each
    /// method is invoked, so we can prove the cache absorbs reads.
    #[derive(Default)]
    struct CountingInner {
        sessions: Mutex<HashMap<PublicKey, spark_wallet::Session>>,
        get_calls: AtomicUsize,
        set_calls: AtomicUsize,
    }

    #[macros::async_trait]
    impl spark_wallet::SessionStore for CountingInner {
        async fn get_session(
            &self,
            key: &PublicKey,
        ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            self.sessions
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or(spark_wallet::SessionStoreError::NotFound)
        }

        async fn set_session(
            &self,
            key: &PublicKey,
            session: spark_wallet::Session,
        ) -> Result<(), spark_wallet::SessionStoreError> {
            self.set_calls.fetch_add(1, Ordering::SeqCst);
            self.sessions.lock().unwrap().insert(*key, session);
            Ok(())
        }
    }

    fn test_pubkey(fill: u8) -> PublicKey {
        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[fill.max(1); 32]).unwrap();
        PublicKey::from_secret_key(&secp, &sk)
    }

    fn future_expiration() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(3600)
    }

    #[async_test_all]
    async fn set_writes_through_then_get_hits_cache() {
        let inner = Arc::new(CountingInner::default());
        let sm = CachingSessionStore::new(inner.clone());
        let pk = test_pubkey(1);

        sm.set_session(
            &pk,
            spark_wallet::Session {
                token: "t".to_string(),
                expiration: future_expiration(),
            },
        )
        .await
        .unwrap();
        assert_eq!(inner.set_calls.load(Ordering::SeqCst), 1);
        assert_eq!(inner.get_calls.load(Ordering::SeqCst), 0);

        // Subsequent gets should hit the cache without touching the inner.
        for _ in 0..3 {
            sm.get_session(&pk).await.unwrap();
        }
        assert_eq!(inner.get_calls.load(Ordering::SeqCst), 0);
    }

    #[async_test_all]
    async fn first_get_populates_cache_on_miss() {
        let inner = Arc::new(CountingInner::default());
        let pk = test_pubkey(2);
        inner.sessions.lock().unwrap().insert(
            pk,
            spark_wallet::Session {
                token: "from-db".to_string(),
                expiration: future_expiration(),
            },
        );
        let sm = CachingSessionStore::new(inner.clone());

        // First read: miss, hits inner.
        let s1 = sm.get_session(&pk).await.unwrap();
        assert_eq!(s1.token, "from-db");
        assert_eq!(inner.get_calls.load(Ordering::SeqCst), 1);

        // Second read: cache hit, no further inner traffic.
        let s2 = sm.get_session(&pk).await.unwrap();
        assert_eq!(s2.token, "from-db");
        assert_eq!(inner.get_calls.load(Ordering::SeqCst), 1);
    }

    #[async_test_all]
    async fn expired_cache_entry_falls_through() {
        let inner = Arc::new(CountingInner::default());
        let pk = test_pubkey(3);
        let sm = CachingSessionStore::new(inner.clone());

        // Seed the cache with an expired session by going through set_session
        // and then mutating the inner store underneath.
        sm.set_session(
            &pk,
            spark_wallet::Session {
                token: "stale".to_string(),
                expiration: 0,
            },
        )
        .await
        .unwrap();
        inner.sessions.lock().unwrap().insert(
            pk,
            spark_wallet::Session {
                token: "fresh".to_string(),
                expiration: future_expiration(),
            },
        );
        let baseline_gets = inner.get_calls.load(Ordering::SeqCst);

        let s = sm.get_session(&pk).await.unwrap();
        assert_eq!(
            s.token, "fresh",
            "expired cache entry must fall through to inner"
        );
        assert_eq!(
            inner.get_calls.load(Ordering::SeqCst),
            baseline_gets.saturating_add(1)
        );
    }

    #[async_test_all]
    async fn missing_inner_propagates_not_found() {
        let inner = Arc::new(CountingInner::default());
        let sm = CachingSessionStore::new(inner);
        let pk = test_pubkey(4);
        let result = sm.get_session(&pk).await;
        assert!(matches!(
            result,
            Err(spark_wallet::SessionStoreError::NotFound)
        ));
    }
}
