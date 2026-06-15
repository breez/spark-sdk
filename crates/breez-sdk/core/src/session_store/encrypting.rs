use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1::PublicKey;

use crate::Network;
use crate::signer::BreezSigner;

use super::SessionStoreError;

/// Hardened derivation indices reserved for session-token encryption.
/// `1397245774` == ASCII "SESN", distinct from `RTSyncSigner`'s indices, so this
/// scope can never collide with another subsystem deriving from the same
/// identity master key.
const ENCRYPTION_DERIVATION_PATH: &str = "m/1397245774'/0'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH_TEST: &str = "m/1397245774'/1'/0'/0/0";

/// Internal decorator that encrypts session tokens at rest via
/// [`BreezSigner::encrypt_ecies`] / [`BreezSigner::decrypt_ecies`], so the
/// underlying [`spark_wallet::SessionStore`] only ever sees ciphertext. Only
/// the `Session::token` field is encrypted; `expiration` stays in plaintext so
/// `is_valid()` can be evaluated cheaply by the caller.
///
/// The receiver keypair is a child of the wallet's `identity_master_key`
/// derived at a fixed path reserved for session-token encryption — distinct
/// from every other subsystem's path. Multiple SDK pods deriving from the
/// same seed therefore share the same key and can decrypt each other's
/// stored tokens; an attacker with read-only DB access cannot.
pub(crate) struct EncryptingSessionStore {
    inner: Arc<dyn spark_wallet::SessionStore>,
    signer: Arc<dyn BreezSigner>,
    encryption_path: DerivationPath,
}

impl EncryptingSessionStore {
    pub(crate) fn new(
        inner: Arc<dyn spark_wallet::SessionStore>,
        signer: Arc<dyn BreezSigner>,
        network: Network,
    ) -> Result<Self, bitcoin::bip32::Error> {
        let encryption_path: DerivationPath = match network {
            Network::Mainnet => ENCRYPTION_DERIVATION_PATH,
            Network::Regtest => ENCRYPTION_DERIVATION_PATH_TEST,
        }
        .parse()?;
        Ok(Self {
            inner,
            signer,
            encryption_path,
        })
    }

    async fn encrypt_token(&self, plaintext: &str) -> Result<String, SessionStoreError> {
        let ciphertext = self
            .signer
            .encrypt_ecies(plaintext.as_bytes(), &self.encryption_path)
            .await
            .map_err(|e| {
                SessionStoreError::Generic(format!("failed to encrypt session token: {e}"))
            })?;
        Ok(BASE64.encode(ciphertext))
    }

    async fn decrypt_token(&self, ciphertext_b64: &str) -> Result<String, SessionStoreError> {
        let ciphertext = BASE64.decode(ciphertext_b64.as_bytes()).map_err(|e| {
            SessionStoreError::Generic(format!("invalid base64 session token: {e}"))
        })?;
        let plaintext = self
            .signer
            .decrypt_ecies(&ciphertext, &self.encryption_path)
            .await
            .map_err(|e| {
                SessionStoreError::Generic(format!("failed to decrypt session token: {e}"))
            })?;
        String::from_utf8(plaintext).map_err(|e| {
            SessionStoreError::Generic(format!("decrypted session token is not utf-8: {e}"))
        })
    }
}

#[macros::async_trait]
impl spark_wallet::SessionStore for EncryptingSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
        let stored = self.inner.get_session(service_identity_key).await?;
        let token = self.decrypt_token(&stored.token).await?;
        Ok(spark_wallet::Session {
            token,
            expiration: stored.expiration,
        })
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: spark_wallet::Session,
    ) -> Result<(), spark_wallet::SessionStoreError> {
        let token = self.encrypt_token(&session.token).await?;
        self.inner
            .set_session(
                service_identity_key,
                spark_wallet::Session {
                    token,
                    expiration: session.expiration,
                },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use macros::async_test_all;
    use spark_wallet::SessionStore as _;

    use crate::Seed;
    use crate::signer::BreezSigner;
    use crate::signer::breez::BreezSignerImpl;

    use super::*;

    /// Trivial in-memory `spark_wallet::SessionStore` used to inspect the
    /// raw bytes the encrypting decorator writes through.
    #[derive(Default)]
    struct InspectableInner {
        sessions: Mutex<HashMap<PublicKey, spark_wallet::Session>>,
    }

    #[macros::async_trait]
    impl spark_wallet::SessionStore for InspectableInner {
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

    fn test_signer(seed_byte: u8) -> Arc<dyn BreezSigner> {
        let seed = Seed::Entropy(vec![seed_byte; 32]);
        let seed_bytes = seed.to_bytes().unwrap();
        let master =
            spark_wallet::identity_master_key(&seed_bytes, Network::Regtest.into(), None).unwrap();
        Arc::new(BreezSignerImpl::new(master))
    }

    #[async_test_all]
    async fn round_trip_decrypts_to_original_token() {
        let inner = Arc::new(InspectableInner::default());
        let signer = test_signer(7);
        let sm = EncryptingSessionStore::new(inner.clone(), signer, Network::Regtest).unwrap();

        let pk = test_pubkey(1);
        let original = "the-bearer-token";
        sm.set_session(
            &pk,
            spark_wallet::Session {
                token: original.to_string(),
                expiration: 1_700_000_000,
            },
        )
        .await
        .unwrap();

        let stored_raw = inner.sessions.lock().unwrap().get(&pk).cloned().unwrap();
        assert_ne!(
            stored_raw.token, original,
            "inner SM must only see ciphertext"
        );
        assert!(BASE64.decode(stored_raw.token.as_bytes()).is_ok());

        let read_back = sm.get_session(&pk).await.unwrap();
        assert_eq!(read_back.token, original);
        assert_eq!(read_back.expiration, 1_700_000_000);
    }

    #[async_test_all]
    async fn distinct_writes_produce_distinct_ciphertext() {
        let inner = Arc::new(InspectableInner::default());
        let signer = test_signer(9);
        let sm = EncryptingSessionStore::new(inner.clone(), signer, Network::Regtest).unwrap();
        let pk = test_pubkey(2);
        let token = "same-plaintext";

        sm.set_session(
            &pk,
            spark_wallet::Session {
                token: token.to_string(),
                expiration: 1,
            },
        )
        .await
        .unwrap();
        let first = inner.sessions.lock().unwrap().get(&pk).cloned().unwrap();

        sm.set_session(
            &pk,
            spark_wallet::Session {
                token: token.to_string(),
                expiration: 2,
            },
        )
        .await
        .unwrap();
        let second = inner.sessions.lock().unwrap().get(&pk).cloned().unwrap();

        assert_ne!(
            first.token, second.token,
            "ECIES with random ephemeral keys must not collide"
        );
    }

    #[async_test_all]
    async fn different_seeds_cannot_decrypt_each_others_tokens() {
        let inner = Arc::new(InspectableInner::default());
        let writer_signer = test_signer(1);
        let reader_signer = test_signer(2);
        let writer =
            EncryptingSessionStore::new(inner.clone(), writer_signer, Network::Regtest).unwrap();
        let reader =
            EncryptingSessionStore::new(inner.clone(), reader_signer, Network::Regtest).unwrap();
        let pk = test_pubkey(3);

        writer
            .set_session(
                &pk,
                spark_wallet::Session {
                    token: "secret".to_string(),
                    expiration: 1,
                },
            )
            .await
            .unwrap();

        let result = reader.get_session(&pk).await;
        let msg = match result {
            Ok(_) => panic!("reader unexpectedly decrypted writer's session"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("decrypt"),
            "expected decrypt error, got: {msg}"
        );
    }
}
