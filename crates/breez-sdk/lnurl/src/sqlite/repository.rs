use lnurl_models::ListMetadataMetadata;
use sqlx::{Row, SqlitePool};

use crate::repository::LnurlSenderComment;
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
            "SELECT pubkey, name, description, nostr_pubkey
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
            nostr_pubkey: row.get(3),
        });
        Ok(maybe_user)
    }

    async fn get_user_by_pubkey(
        &self,
        domain: &str,
        pubkey: &str,
    ) -> Result<Option<User>, LnurlRepositoryError> {
        let maybe_user = sqlx::query(
            "SELECT pubkey, name, description, nostr_pubkey
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
            nostr_pubkey: row.get(3),
        });
        Ok(maybe_user)
    }

    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "REPLACE INTO users (domain, pubkey, name, description, nostr_pubkey, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&user.domain)
        .bind(&user.pubkey)
        .bind(&user.name)
        .bind(&user.description)
        .bind(&user.nostr_pubkey)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "REPLACE INTO zaps (payment_hash, zap_request, zap_event, user_pubkey, invoice_expiry)
            VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&zap.payment_hash)
        .bind(&zap.zap_request)
        .bind(&zap.zap_event)
        .bind(&zap.user_pubkey)
        .bind(zap.invoice_expiry)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_zap_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Zap>, LnurlRepositoryError> {
        let maybe_zap = sqlx::query(
            "SELECT payment_hash, zap_request, zap_event, user_pubkey, invoice_expiry
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
            user_pubkey: row.get(3),
            invoice_expiry: row.get(4),
        });
        Ok(maybe_zap)
    }

    async fn get_users_with_unexpired_invoices(&self) -> Result<Vec<String>, LnurlRepositoryError> {
        let now = now();
        let rows = sqlx::query(
            "SELECT DISTINCT user_pubkey
             FROM zaps
             WHERE invoice_expiry > $1 AND zap_event IS NULL",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let keys = rows.into_iter().map(|row| row.get(0)).collect();
        Ok(keys)
    }

    async fn user_has_unexpired_invoices(
        &self,
        user_pubkey: &str,
    ) -> Result<bool, LnurlRepositoryError> {
        let now = now();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM zaps
             WHERE user_pubkey = $1 AND invoice_expiry > $2 AND zap_event IS NULL",
        )
        .bind(user_pubkey)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO sender_comments (payment_hash, user_pubkey, sender_comment, invoice_expiry)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(payment_hash) DO UPDATE
             SET user_pubkey = excluded.user_pubkey
             ,   sender_comment = excluded.sender_comment
             ,   invoice_expiry = excluded.invoice_expiry",
        )
        .bind(&comment.payment_hash)
        .bind(&comment.user_pubkey)
        .bind(&comment.comment)
        .bind(comment.invoice_expiry)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_metadata_by_pubkey(
        &self,
        pubkey: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ListMetadataMetadata>, LnurlRepositoryError> {
        let rows = sqlx::query(
            "SELECT COALESCE(z.payment_hash, sc.payment_hash) AS payment_hash
             ,      sc.sender_comment
             ,      z.zap_request
             FROM zaps z
             FULL JOIN sender_comments sc ON z.payment_hash = sc.payment_hash
             WHERE z.user_pubkey = $1 OR sc.user_pubkey = $1
             ORDER BY COALESCE(z.invoice_expiry, sc.invoice_expiry) ASC
             LIMIT $3 OFFSET $2",
        )
        .bind(pubkey)
        .bind(i64::from(offset))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let metadata = rows
            .into_iter()
            .map(|row| ListMetadataMetadata {
                payment_hash: row.get(0),
                sender_comment: row.get(1),
                nostr_zap_request: row.get(2),
            })
            .collect();
        Ok(metadata)
    }
}
