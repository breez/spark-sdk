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
    async fn delete_user(&self, pubkey: &str) -> Result<(), LnurlRepositoryError> {
        sqlx::query("DELETE FROM users WHERE pubkey = ?")
            .bind(pubkey)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_user_by_name(&self, name: &str) -> Result<Option<User>, LnurlRepositoryError> {
        let maybe_user = sqlx::query("SELECT pubkey, name, description FROM users WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| User {
                pubkey: row.get(0),
                name: row.get(1),
                description: row.get(2),
            });
        Ok(maybe_user)
    }

    async fn get_user_by_pubkey(&self, pubkey: &str) -> Result<Option<User>, LnurlRepositoryError> {
        let maybe_user =
            sqlx::query("SELECT pubkey, name, description FROM users WHERE pubkey = ?")
                .bind(pubkey)
                .fetch_optional(&self.pool)
                .await?
                .map(|row| User {
                    pubkey: row.get(0),
                    name: row.get(1),
                    description: row.get(2),
                });
        Ok(maybe_user)
    }

    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError> {
        sqlx::query("INSERT INTO users (pubkey, name, description) VALUES (?, ?, ?) ON CONFLICT(pubkey) DO UPDATE SET name = excluded.name, description = excluded.description")
            .bind(&user.pubkey)
            .bind(&user.name)
            .bind(&user.description)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
