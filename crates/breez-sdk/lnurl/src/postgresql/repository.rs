use lnurl_models::ListInvoicesInvoice;
use sqlx::{PgPool, Row};

use crate::{
    repository::{LnurlRepositoryError, LnurlSenderComment, ZapRequest},
    time::now,
    user::User,
};

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
            "INSERT INTO users (domain, pubkey, name, description, nostr_pubkey, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(domain, pubkey) DO UPDATE
             SET name = excluded.name, description = excluded.description, nostr_pubkey = excluded.nostr_pubkey, updated_at = excluded.updated_at",
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

    async fn insert_nostr_zap_request(
        &self,
        zap_request: &ZapRequest,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO nostr_zap_requests (user_pubkey, invoice, zap_request, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&zap_request.user_pubkey)
        .bind(&zap_request.invoice)
        .bind(&zap_request.zap_request)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO lnurl_sender_comments (user_pubkey, invoice, comment, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&comment.user_pubkey)
        .bind(&comment.invoice)
        .bind(&comment.comment)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_invoices_by_pubkey(
        &self,
        pubkey: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ListInvoicesInvoice>, LnurlRepositoryError> {
        let invoices = sqlx::query(
            "SELECT COALESCE(c.invoice, n.invoice) AS invoice
            ,       c.comment
            ,       n.zap_request
             FROM lnurl_sender_comments c
             FULL OUTER JOIN nostr_zap_requests n
                ON c.invoice = n.invoice
             WHERE c.user_pubkey = $1 OR n.user_pubkey = $1
             ORDER BY COALESCE(c.created_at, n.created_at), c.invoice
             LIMIT $2 OFFSET $3",
        )
        .bind(pubkey)
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok(ListInvoicesInvoice {
                invoice: row.try_get(0)?,
                sender_comment: row.try_get(1)?,
                nostr_zap_request: row.try_get(2)?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(invoices)
    }
}
