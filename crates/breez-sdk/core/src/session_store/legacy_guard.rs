use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;

/// Prefixes a prior SDK version tagged onto stored tokens when it encrypted them
/// (`enc:`) or stored them in plaintext (`pln:`). This version stores tokens
/// verbatim and does not recognize them.
const LEGACY_MARKERS: [&str; 2] = ["enc:", "pln:"];

/// Transitional decorator that hides session tokens written by a prior SDK
/// version, which encrypted and tag-prefixed them.
///
/// This version stores tokens verbatim, so a legacy-tagged token read back would
/// be sent to the server as-is, rejected, and (auth failures do not invalidate
/// the cache) keep failing until its TTL. Reading it as absent instead forces a
/// clean re-auth that overwrites it with a raw token, so a mixed-version
/// deployment self-heals per user on first access rather than via a one-shot
/// wipe. A real SSP/operator token (base64 / JWT) never carries these prefixes.
///
/// Removable once no legacy-format rows can remain in any deployment.
pub(crate) struct LegacyTokenGuard {
    inner: Arc<dyn spark_wallet::SessionStore>,
}

impl LegacyTokenGuard {
    pub(crate) fn new(inner: Arc<dyn spark_wallet::SessionStore>) -> Self {
        Self { inner }
    }
}

#[macros::async_trait]
impl spark_wallet::SessionStore for LegacyTokenGuard {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
        let session = self.inner.get_session(service_identity_key).await?;
        if LEGACY_MARKERS
            .iter()
            .any(|marker| session.token.starts_with(marker))
        {
            return Err(spark_wallet::SessionStoreError::NotFound);
        }
        Ok(session)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: spark_wallet::Session,
    ) -> Result<(), spark_wallet::SessionStoreError> {
        self.inner.set_session(service_identity_key, session).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use macros::async_test_all;
    use spark_wallet::SessionStore as _;

    use super::*;

    #[derive(Default)]
    struct InMemoryInner {
        sessions: Mutex<HashMap<PublicKey, spark_wallet::Session>>,
    }

    #[macros::async_trait]
    impl spark_wallet::SessionStore for InMemoryInner {
        async fn get_session(
            &self,
            key: &PublicKey,
        ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
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

    #[async_test_all]
    async fn legacy_tagged_tokens_read_as_absent() {
        let inner = Arc::new(InMemoryInner::default());
        let guard = LegacyTokenGuard::new(inner.clone());
        for (fill, marker) in [(1u8, "enc:"), (2u8, "pln:")] {
            let pk = test_pubkey(fill);
            inner.sessions.lock().unwrap().insert(
                pk,
                spark_wallet::Session {
                    token: format!("{marker}deadbeef"),
                    expiration: u64::MAX,
                },
            );
            assert!(matches!(
                guard.get_session(&pk).await,
                Err(spark_wallet::SessionStoreError::NotFound)
            ));
        }
    }

    #[async_test_all]
    async fn raw_tokens_pass_through() {
        let inner = Arc::new(InMemoryInner::default());
        let guard = LegacyTokenGuard::new(inner.clone());
        let pk = test_pubkey(3);
        guard
            .set_session(
                &pk,
                spark_wallet::Session {
                    token: "raw-bearer-token".to_string(),
                    expiration: u64::MAX,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            guard.get_session(&pk).await.unwrap().token,
            "raw-bearer-token"
        );
    }
}
