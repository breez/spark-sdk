use sqlx::{Row, SqlitePool};

use crate::zap::Zap;
use crate::{repository::LnurlRepositoryError, time::now, user::User};

#[derive(Clone)]
pub struct LnurlRepository {
    pool: SqlitePool,
}

impl LnurlRepository {
    pub fn new(pool: SqlitePool) -> Self {
        LnurlRepository { pool }
    }
}

#[async_trait::async_trait]
impl crate::repository::LnurlRepository for LnurlRepository {
    async fn delete_user(&self, domain: &str, pubkey: &str) -> Result<(), LnurlRepositoryError> {
        sqlx::query("DELETE FROM users WHERE domain = $1 AND pubkey = $2")
            .bind(domain)
            .bind(pubkey)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_user_by_name(
        &self,
        domain: &str,
        name: &str,
    ) -> Result<Option<User>, LnurlRepositoryError> {
        let maybe_user = sqlx::query(
            "SELECT pubkey, name, description 
            FROM users 
            WHERE domain = $1 AND name = $2",
        )
        .bind(domain)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| User {
            domain: domain.to_string(),
            pubkey: row.get(0),
            name: row.get(1),
            description: row.get(2),
        });
        Ok(maybe_user)
    }

    async fn get_user_by_pubkey(
        &self,
        domain: &str,
        pubkey: &str,
    ) -> Result<Option<User>, LnurlRepositoryError> {
        let maybe_user = sqlx::query(
            "SELECT pubkey, name, description
                FROM users
                WHERE domain = $1 AND pubkey = $2",
        )
        .bind(domain)
        .bind(pubkey)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| User {
            domain: domain.to_string(),
            pubkey: row.get(0),
            name: row.get(1),
            description: row.get(2),
        });
        Ok(maybe_user)
    }

    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "REPLACE INTO users (domain, pubkey, name, description, updated_at)
            VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&user.domain)
        .bind(&user.pubkey)
        .bind(&user.name)
        .bind(&user.description)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_user_keys(&self) -> Result<Vec<String>, LnurlRepositoryError> {
        let rows = sqlx::query("SELECT DISTINCT pubkey FROM users")
            .fetch_all(&self.pool)
            .await?;
        let keys = rows.into_iter().map(|row| row.get(0)).collect();
        Ok(keys)
    }

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "REPLACE INTO zaps (payment_hash, zap_request, zap_event)
            VALUES ($1, $2, $3)",
        )
        .bind(&zap.payment_hash)
        .bind(&zap.zap_request)
        .bind(&zap.zap_event)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_zap_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Zap>, LnurlRepositoryError> {
        let maybe_zap = sqlx::query(
            "SELECT payment_hash, zap_request, zap_event
                FROM zaps
                WHERE payment_hash = $1",
        )
        .bind(payment_hash)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| Zap {
            payment_hash: row.get(0),
            zap_request: row.get(1),
            zap_event: row.get(2),
        });
        Ok(maybe_zap)
    }
}
