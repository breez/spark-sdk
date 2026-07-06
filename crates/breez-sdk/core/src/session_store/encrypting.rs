use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use bitcoin::bip32::DerivationPath;
use bitcoin::secp256k1::PublicKey;

use crate::Network;
use crate::signer::EciesSigner;

use super::SessionStoreError;

/// Prefixes recording which mode a stored token was written in, so a mode switch
/// (encryption enabled or disabled between runs) is caught on read.
const MARKER_ENCRYPTED: &str = "enc:";
const MARKER_PLAINTEXT: &str = "pln:";

/// Hardened derivation indices reserved for session-token encryption.
/// `1397245774` == ASCII "SESN", distinct from `RTSyncSigner`'s indices, so this
/// scope can never collide with another subsystem deriving from the same
/// identity master key.
const ENCRYPTION_DERIVATION_PATH: &str = "m/1397245774'/0'/0'/0/0";
const ENCRYPTION_DERIVATION_PATH_TEST: &str = "m/1397245774'/1'/0'/0/0";

/// `SessionStore` decorator for session tokens. When an [`EciesSigner`] is
/// present it encrypts the token at rest via [`EciesSigner::encrypt_ecies`] /
/// [`EciesSigner::decrypt_ecies`]; otherwise (a signing-only signer) it stores
/// the token in plaintext. Only `Session::token` is transformed; `expiration`
/// stays plaintext so `is_valid()` stays cheap.
///
/// Each stored token is tagged with its mode (`enc:` / `pln:`). On read, a token
/// written in the other mode (or a legacy untagged one) reads as `NotFound`, so
/// switching the signer's ECIES capability on an existing wallet forces a
/// re-authentication that re-stores the token in the current mode. Detection is
/// symmetric: a stale ciphertext and a stale plaintext token are both rejected in
/// the other mode.
///
/// The encryption keypair is a child of the wallet's `identity_master_key` at a
/// fixed reserved path. SDK pods sharing the seed share the key and can decrypt
/// each other's tokens; an attacker with read-only DB access cannot.
pub(crate) struct EncryptingSessionStore {
    inner: Arc<dyn spark_wallet::SessionStore>,
    ecies: Option<Arc<dyn EciesSigner>>,
    encryption_path: DerivationPath,
}

impl EncryptingSessionStore {
    pub(crate) fn new(
        inner: Arc<dyn spark_wallet::SessionStore>,
        ecies: Option<Arc<dyn EciesSigner>>,
        network: Network,
    ) -> Result<Self, bitcoin::bip32::Error> {
        let encryption_path: DerivationPath = match network {
            Network::Mainnet => ENCRYPTION_DERIVATION_PATH,
            Network::Regtest => ENCRYPTION_DERIVATION_PATH_TEST,
        }
        .parse()?;
        Ok(Self {
            inner,
            ecies,
            encryption_path,
        })
    }

    /// Tags `token` for storage in the current mode: `enc:` + ciphertext when an
    /// ECIES signer is present, `pln:` + the token otherwise. Encryption failure
    /// propagates (fail closed: never falls back to plaintext).
    async fn encode_token(&self, token: &str) -> Result<String, SessionStoreError> {
        if let Some(ecies) = &self.ecies {
            let ciphertext = ecies
                .encrypt_ecies(token.as_bytes(), &self.encryption_path)
                .await
                .map_err(|e| {
                    SessionStoreError::Generic(format!("failed to encrypt session token: {e}"))
                })?;
            Ok(format!("{MARKER_ENCRYPTED}{}", BASE64.encode(ciphertext)))
        } else {
            Ok(format!("{MARKER_PLAINTEXT}{token}"))
        }
    }

    /// The usable token if `stored` was written in the current mode, else `None`
    /// (a mode switch, a legacy untagged token, or an undecryptable one). `None`
    /// surfaces as `NotFound`.
    async fn decode_token(&self, stored: &str) -> Option<String> {
        if let Some(ecies) = &self.ecies {
            let ciphertext = BASE64.decode(stored.strip_prefix(MARKER_ENCRYPTED)?).ok()?;
            let plaintext = ecies
                .decrypt_ecies(&ciphertext, &self.encryption_path)
                .await
                .ok()?;
            String::from_utf8(plaintext).ok()
        } else {
            stored.strip_prefix(MARKER_PLAINTEXT).map(str::to_string)
        }
    }
}

#[macros::async_trait]
impl spark_wallet::SessionStore for EncryptingSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<spark_wallet::Session, spark_wallet::SessionStoreError> {
        let stored = self.inner.get_session(service_identity_key).await?;
        // A token written in the other mode (or a legacy untagged one) is treated
        // as absent, so the caller re-authenticates and re-stores it in the
        // current mode.
        let Some(token) = self.decode_token(&stored.token).await else {
            return Err(spark_wallet::SessionStoreError::NotFound);
        };
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
        let token = self.encode_token(&session.token).await?;
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

    use bitcoin::bip32::DerivationPath;
    use macros::async_test_all;
    use spark_wallet::SessionStore as _;

    use crate::SdkError;
    use crate::Seed;
    use crate::signer::EciesSigner;
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

    fn test_signer(seed_byte: u8) -> Arc<dyn EciesSigner> {
        let seed = Seed::Entropy(vec![seed_byte; 32]);
        let seed_bytes = seed.to_bytes().unwrap();
        let master =
            spark_wallet::identity_master_key(&seed_bytes, Network::Regtest.into(), None).unwrap();
        Arc::new(BreezSignerImpl::new(master))
    }

    fn store(inner: Arc<InspectableInner>, seed: u8, encrypt: bool) -> EncryptingSessionStore {
        let ecies = encrypt.then(|| test_signer(seed));
        EncryptingSessionStore::new(inner, ecies, Network::Regtest).unwrap()
    }

    #[async_test_all]
    async fn encrypt_mode_round_trips_and_tags_ciphertext() {
        let inner = Arc::new(InspectableInner::default());
        let sm = store(inner.clone(), 7, true);

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
        assert!(
            stored_raw.token.starts_with(MARKER_ENCRYPTED),
            "encrypt mode must tag with the enc: marker"
        );
        assert_ne!(
            stored_raw.token, original,
            "inner store must not see plaintext"
        );
        assert!(
            BASE64
                .decode(stored_raw.token.strip_prefix(MARKER_ENCRYPTED).unwrap())
                .is_ok()
        );

        let read_back = sm.get_session(&pk).await.unwrap();
        assert_eq!(read_back.token, original);
        assert_eq!(read_back.expiration, 1_700_000_000);
    }

    #[async_test_all]
    async fn plaintext_mode_round_trips_and_tags_token() {
        let inner = Arc::new(InspectableInner::default());
        let sm = store(inner.clone(), 8, false);

        let pk = test_pubkey(8);
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
        assert_eq!(stored_raw.token, format!("{MARKER_PLAINTEXT}{original}"));
        assert_eq!(sm.get_session(&pk).await.unwrap().token, original);
    }

    #[async_test_all]
    async fn distinct_writes_produce_distinct_ciphertext() {
        let inner = Arc::new(InspectableInner::default());
        let sm = store(inner.clone(), 9, true);
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
        let writer = store(inner.clone(), 1, true);
        let reader = store(inner.clone(), 2, true);
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

        // A wrong-key reader can't decrypt the token, so it reads as absent
        // (NotFound) and the caller re-authenticates instead of using a bad token.
        assert!(
            matches!(
                reader.get_session(&pk).await,
                Err(spark_wallet::SessionStoreError::NotFound)
            ),
            "expected NotFound for an undecryptable token"
        );
    }

    /// Mode switch plaintext -> encrypt: a token stored in plaintext mode reads
    /// as `NotFound` once encryption is enabled, forcing a re-auth that re-stores
    /// it encrypted.
    #[async_test_all]
    async fn plaintext_token_read_in_encrypt_mode_is_not_found() {
        let inner = Arc::new(InspectableInner::default());
        let pk = test_pubkey(5);
        store(inner.clone(), 5, false)
            .set_session(
                &pk,
                spark_wallet::Session {
                    token: "the-bearer-token".to_string(),
                    expiration: 2_000_000_000,
                },
            )
            .await
            .unwrap();

        assert!(
            matches!(
                store(inner.clone(), 5, true).get_session(&pk).await,
                Err(spark_wallet::SessionStoreError::NotFound)
            ),
            "a plaintext-mode token must read as NotFound in encrypt mode"
        );
    }

    /// Mode switch encrypt -> plaintext: a ciphertext stored in encrypt mode
    /// reads as `NotFound` after a downgrade, the symmetric counterpart the marker
    /// adds (previously it was returned verbatim and only recovered on expiry).
    #[async_test_all]
    async fn ciphertext_read_in_plaintext_mode_is_not_found() {
        let inner = Arc::new(InspectableInner::default());
        let pk = test_pubkey(6);
        store(inner.clone(), 6, true)
            .set_session(
                &pk,
                spark_wallet::Session {
                    token: "the-bearer-token".to_string(),
                    expiration: 2_000_000_000,
                },
            )
            .await
            .unwrap();

        assert!(
            matches!(
                store(inner.clone(), 6, false).get_session(&pk).await,
                Err(spark_wallet::SessionStoreError::NotFound)
            ),
            "a ciphertext token must read as NotFound in plaintext mode"
        );
    }

    /// An `EciesSigner` that cannot encrypt, modeling a signer whose
    /// encryption-key export is denied.
    struct NoEncryptSigner;

    #[macros::async_trait]
    impl EciesSigner for NoEncryptSigner {
        async fn encrypt_ecies(
            &self,
            _message: &[u8],
            _path: &DerivationPath,
        ) -> Result<Vec<u8>, SdkError> {
            Err(SdkError::Signer("encryption-key export denied".to_string()))
        }
        async fn decrypt_ecies(
            &self,
            _message: &[u8],
            _path: &DerivationPath,
        ) -> Result<Vec<u8>, SdkError> {
            Err(SdkError::Signer("encryption-key export denied".to_string()))
        }
    }

    /// Fail-closed: when the signer cannot encrypt the token (a no-export signer
    /// under a deny policy), `set_session` errors and the inner store receives
    /// nothing. It must never fall back to writing the plaintext token.
    #[async_test_all]
    async fn encrypt_failure_never_writes_plaintext() {
        let inner = Arc::new(InspectableInner::default());
        let signer: Arc<dyn EciesSigner> = Arc::new(NoEncryptSigner);
        let sm =
            EncryptingSessionStore::new(inner.clone(), Some(signer), Network::Regtest).unwrap();
        let pk = test_pubkey(4);

        let result = sm
            .set_session(
                &pk,
                spark_wallet::Session {
                    token: "plaintext-bearer-token".to_string(),
                    expiration: 1_700_000_000,
                },
            )
            .await;

        assert!(
            result.is_err(),
            "set_session must fail when the signer cannot encrypt"
        );
        assert!(
            inner.sessions.lock().unwrap().is_empty(),
            "fail closed: nothing may be written to the inner store when encryption fails"
        );
    }
}
