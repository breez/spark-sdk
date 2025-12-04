use lnurl_models::ListMetadataMetadata;
use sqlx::{PgPool, Row};

use crate::repository::LnurlSenderComment;
use crate::zap::Zap;
use crate::{repository::LnurlRepositoryError, time::now, user::User};

#[derive(Clone)]
pub struct LnurlRepository {
    pool: PgPool,
}

impl LnurlRepository {
    pub fn new(pool: PgPool) -> Self {
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
        .map(|row| {
            Ok::<_, sqlx::Error>(User {
                domain: domain.to_string(),
                pubkey: row.try_get(0)?,
                name: row.try_get(1)?,
                description: row.try_get(2)?,
                nostr_pubkey: row.try_get(3)?,
            })
        })
        .transpose()?;
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
        .map(|row| {
            Ok::<_, sqlx::Error>(User {
                domain: domain.to_string(),
                pubkey: row.try_get(0)?,
                name: row.try_get(1)?,
                description: row.try_get(2)?,
                nostr_pubkey: row.try_get(3)?,
            })
        })
        .transpose()?;
        Ok(maybe_user)
    }

    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO users (domain, pubkey, name, description, nostr_pubkey, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(domain, pubkey) DO UPDATE
             SET name = excluded.name
             ,   description = excluded.description
             ,   nostr_pubkey = excluded.nostr_pubkey
             ,   updated_at = excluded.updated_at",
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
            "INSERT INTO zaps (payment_hash, zap_request, zap_event
            , user_pubkey, invoice_expiry, updated_at, is_user_nostr_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT(payment_hash) DO UPDATE
             SET zap_request = excluded.zap_request
             ,   zap_event = excluded.zap_event
             ,   user_pubkey = excluded.user_pubkey
             ,   invoice_expiry = excluded.invoice_expiry
             ,   updated_at = excluded.updated_at
             ,   is_user_nostr_key = excluded.is_user_nostr_key",
        )
        .bind(&zap.payment_hash)
        .bind(&zap.zap_request)
        .bind(&zap.zap_event)
        .bind(&zap.user_pubkey)
        .bind(zap.invoice_expiry)
        .bind(zap.updated_at)
        .bind(zap.is_user_nostr_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_zap_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Zap>, LnurlRepositoryError> {
        let maybe_zap = sqlx::query(
            "SELECT payment_hash, zap_request, zap_event, user_pubkey
            , invoice_expiry, updated_at, is_user_nostr_key
             FROM zaps
             WHERE payment_hash = $1",
        )
        .bind(payment_hash)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| {
            Ok::<_, sqlx::Error>(Zap {
                payment_hash: row.try_get(0)?,
                zap_request: row.try_get(1)?,
                zap_event: row.try_get(2)?,
                user_pubkey: row.try_get(3)?,
                invoice_expiry: row.try_get(4)?,
                updated_at: row.try_get(5)?,
                is_user_nostr_key: row.try_get(6)?,
            })
        })
        .transpose()?;
        Ok(maybe_zap)
    }

    async fn get_zap_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError> {
        let now = now();
        let rows = sqlx::query(
            "SELECT DISTINCT user_pubkey
             FROM zaps
             WHERE invoice_expiry > $1 AND zap_event IS NULL AND is_user_nostr_key = FALSE",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let keys = rows
            .into_iter()
            .map(|row| row.try_get(0))
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(keys)
    }

    async fn is_zap_monitored_user(&self, user_pubkey: &str) -> Result<bool, LnurlRepositoryError> {
        let now = now();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM zaps
             WHERE user_pubkey = $1 AND invoice_expiry > $2 AND zap_event IS NULL AND is_user_nostr_key = FALSE",
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
            "INSERT INTO sender_comments (payment_hash, user_pubkey, sender_comment, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(payment_hash) DO UPDATE
             SET user_pubkey = excluded.user_pubkey
             ,   sender_comment = excluded.sender_comment
             ,   updated_at = excluded.updated_at",
        )
        .bind(&comment.payment_hash)
        .bind(&comment.user_pubkey)
        .bind(&comment.comment)
        .bind(comment.updated_at)
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
             ,      z.zap_event
             ,      COALESCE(z.updated_at, sc.updated_at) AS updated_at
             FROM zaps z
             FULL JOIN sender_comments sc ON z.payment_hash = sc.payment_hash
             WHERE z.user_pubkey = $1 OR sc.user_pubkey = $1
             ORDER BY COALESCE(z.updated_at, sc.updated_at) ASC
             OFFSET $2 LIMIT $3",
        )
        .bind(pubkey)
        .bind(i64::from(offset))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let metadata = rows
            .into_iter()
            .map(|row| {
                Ok(ListMetadataMetadata {
                    payment_hash: row.try_get(0)?,
                    sender_comment: row.try_get(1)?,
                    nostr_zap_request: row.try_get(2)?,
                    nostr_zap_receipt: row.try_get(3)?,
                    updated_at: row.try_get(4)?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(metadata)
    }

    async fn list_domains(&self) -> Result<Vec<String>, LnurlRepositoryError> {
        let rows = sqlx::query("SELECT domain FROM allowed_domains")
            .fetch_all(&self.pool)
            .await?;

        let domains = rows
            .into_iter()
            .map(|row| row.try_get(0))
            .collect::<Result<Vec<String>, sqlx::Error>>()?;

        Ok(domains)
    }

    async fn add_domain(&self, domain: &str) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO allowed_domains (domain)
             VALUES ($1)
             ON CONFLICT(domain) DO NOTHING",
        )
        .bind(domain)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
