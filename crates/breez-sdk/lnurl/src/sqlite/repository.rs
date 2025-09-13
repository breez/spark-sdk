use sqlx::{Row, SqlitePool};

use crate::{repository::LnurlRepositoryError, user::User};

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
            "REPLACE INTO users (domain, pubkey, name, description)
            VALUES ($1, $2, $3, $4)",
        )
        .bind(&user.domain)
        .bind(&user.pubkey)
        .bind(&user.name)
        .bind(&user.description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
