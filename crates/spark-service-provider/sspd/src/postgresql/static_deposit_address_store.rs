use std::sync::Arc;

use async_trait::async_trait;
use bitcoin::secp256k1::PublicKey;
use deadpool_postgres::Pool;

use super::{execute, query_opt};
use crate::static_deposit::repository::{StaticDepositAddress, StaticDepositAddressStore};

pub struct PostgresStaticDepositAddressStore {
    pool: Arc<Pool>,
}

impl PostgresStaticDepositAddressStore {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StaticDepositAddressStore for PostgresStaticDepositAddressStore {
    async fn get(&self, address: &str) -> Result<Option<StaticDepositAddress>, String> {
        let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = query_opt(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r"SELECT user_identity_public_key, user_signing_public_key, verifying_public_key
               FROM brz_ssp_static_deposit_addresses
               WHERE address = $1",
            &[&address],
        )
        .await
        .and_then(|row| {
            row.map(|row| Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?)))
                .transpose()
        })
        .map_err(|e| e.to_string())?;
        row.map(|(identity, signing, verifying)| {
            Ok(StaticDepositAddress {
                address: address.to_string(),
                user_identity_public_key: PublicKey::from_slice(&identity)
                    .map_err(|e| format!("invalid identity key: {e}"))?,
                user_signing_public_key: PublicKey::from_slice(&signing)
                    .map_err(|e| format!("invalid signing key: {e}"))?,
                verifying_public_key: PublicKey::from_slice(&verifying)
                    .map_err(|e| format!("invalid verifying key: {e}"))?,
            })
        })
        .transpose()
    }

    async fn insert(&self, address: &StaticDepositAddress) -> Result<(), String> {
        execute(
            &self.pool.get().await.map_err(|e| e.to_string())?,
            r"INSERT INTO brz_ssp_static_deposit_addresses
                   (address, user_identity_public_key, user_signing_public_key,
                    verifying_public_key)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (address) DO NOTHING",
            &[
                &address.address,
                &address.user_identity_public_key.serialize().as_slice(),
                &address.user_signing_public_key.serialize().as_slice(),
                &address.verifying_public_key.serialize().as_slice(),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgresql::testing::TestDatabase;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};

    fn key(byte: u8) -> PublicKey {
        SecretKey::from_slice(&[byte; 32])
            .unwrap()
            .public_key(&Secp256k1::new())
    }

    #[tokio::test]
    async fn an_address_keeps_the_user_and_keys_it_was_first_stored_with() {
        let database = TestDatabase::new().await;
        let pool = Arc::clone(&database.pool);
        let store = PostgresStaticDepositAddressStore::new(pool);
        let first = StaticDepositAddress {
            address: "bcrt1first".to_string(),
            user_identity_public_key: key(1),
            user_signing_public_key: key(2),
            verifying_public_key: key(3),
        };
        assert_eq!(store.get(&first.address).await.unwrap(), None);

        store.insert(&first).await.unwrap();
        let other = StaticDepositAddress {
            user_identity_public_key: key(4),
            user_signing_public_key: key(5),
            verifying_public_key: key(6),
            ..first.clone()
        };
        store.insert(&other).await.unwrap();

        assert_eq!(store.get(&first.address).await.unwrap(), Some(first));
    }
}
