use lnurl_models::ListMetadataMetadata;
use sqlx::{PgPool, Row};

use crate::repository::{Invoice, LnurlSenderComment, NewlyPaid};
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
            "SELECT pubkey, name, description, nostr_pubkey, no_invoice_paid_support
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
                no_invoice_paid_support: row.try_get(4)?,
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
            "SELECT pubkey, name, description, nostr_pubkey, no_invoice_paid_support
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
                no_invoice_paid_support: row.try_get(4)?,
            })
        })
        .transpose()?;
        Ok(maybe_user)
    }

    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO users (domain, pubkey, name, description, nostr_pubkey, no_invoice_paid_support, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT(domain, pubkey) DO UPDATE
             SET name = excluded.name
             ,   description = excluded.description
             ,   nostr_pubkey = excluded.nostr_pubkey
             ,   no_invoice_paid_support = excluded.no_invoice_paid_support
             ,   updated_at = excluded.updated_at",
        )
        .bind(&user.domain)
        .bind(&user.pubkey)
        .bind(&user.name)
        .bind(&user.description)
        .bind(&user.nostr_pubkey)
        .bind(user.no_invoice_paid_support)
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
        updated_after: Option<i64>,
    ) -> Result<Vec<ListMetadataMetadata>, LnurlRepositoryError> {
        let updated_after = updated_after.unwrap_or(0);
        let rows = sqlx::query(
            "SELECT COALESCE(z.payment_hash, sc.payment_hash, i.payment_hash) AS payment_hash
             ,      sc.sender_comment
             ,      z.zap_request
             ,      z.zap_event
             ,      COALESCE(z.updated_at, sc.updated_at, i.updated_at) AS updated_at
             ,      i.preimage
             FROM invoices i
             LEFT JOIN zaps z ON i.payment_hash = z.payment_hash
             LEFT JOIN sender_comments sc ON i.payment_hash = sc.payment_hash
             WHERE i.user_pubkey = $1 AND COALESCE(z.updated_at, sc.updated_at, i.updated_at) > $4
             UNION
             SELECT COALESCE(z.payment_hash, sc.payment_hash) AS payment_hash
             ,      sc.sender_comment
             ,      z.zap_request
             ,      z.zap_event
             ,      COALESCE(z.updated_at, sc.updated_at) AS updated_at
             ,      NULL as preimage
             FROM zaps z
             FULL JOIN sender_comments sc ON z.payment_hash = sc.payment_hash
             LEFT JOIN invoices i ON COALESCE(z.payment_hash, sc.payment_hash) = i.payment_hash
             WHERE (z.user_pubkey = $1 OR sc.user_pubkey = $1) AND i.payment_hash IS NULL
               AND COALESCE(z.updated_at, sc.updated_at) > $4
             ORDER BY updated_at ASC
             OFFSET $2 LIMIT $3",
        )
        .bind(pubkey)
        .bind(i64::from(offset))
        .bind(i64::from(limit))
        .bind(updated_after)
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
                    preimage: row.try_get(5)?,
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

    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO invoices (payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT(payment_hash) DO UPDATE
             SET user_pubkey = excluded.user_pubkey
             ,   invoice = excluded.invoice
             ,   preimage = excluded.preimage
             ,   invoice_expiry = excluded.invoice_expiry
             ,   updated_at = excluded.updated_at",
        )
        .bind(&invoice.payment_hash)
        .bind(&invoice.user_pubkey)
        .bind(&invoice.invoice)
        .bind(&invoice.preimage)
        .bind(invoice.invoice_expiry)
        .bind(invoice.created_at)
        .bind(invoice.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Invoice>, LnurlRepositoryError> {
        let maybe_invoice = sqlx::query(
            "SELECT payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at
             FROM invoices
             WHERE payment_hash = $1",
        )
        .bind(payment_hash)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| {
            Ok::<_, sqlx::Error>(Invoice {
                payment_hash: row.try_get(0)?,
                user_pubkey: row.try_get(1)?,
                invoice: row.try_get(2)?,
                preimage: row.try_get(3)?,
                invoice_expiry: row.try_get(4)?,
                created_at: row.try_get(5)?,
                updated_at: row.try_get(6)?,
            })
        })
        .transpose()?;
        Ok(maybe_invoice)
    }

    async fn get_invoice_monitored_users(&self) -> Result<Vec<String>, LnurlRepositoryError> {
        let now = now();
        let rows = sqlx::query(
            "SELECT DISTINCT i.user_pubkey
             FROM invoices i
             JOIN users u ON i.user_pubkey = u.pubkey
             WHERE i.invoice_expiry > $1 AND i.preimage IS NULL AND u.no_invoice_paid_support = FALSE",
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

    async fn is_invoice_monitored_user(
        &self,
        user_pubkey: &str,
    ) -> Result<bool, LnurlRepositoryError> {
        let now = now();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM invoices i
             JOIN users u ON i.user_pubkey = u.pubkey
             WHERE i.user_pubkey = $1 AND i.invoice_expiry > $2 AND i.preimage IS NULL AND u.no_invoice_paid_support = FALSE",
        )
        .bind(user_pubkey)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    async fn insert_newly_paid(&self, newly_paid: &NewlyPaid) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO newly_paid (payment_hash, created_at, retry_count, next_retry_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(payment_hash) DO NOTHING",
        )
        .bind(&newly_paid.payment_hash)
        .bind(newly_paid.created_at)
        .bind(newly_paid.retry_count)
        .bind(newly_paid.next_retry_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_pending_newly_paid(&self) -> Result<Vec<NewlyPaid>, LnurlRepositoryError> {
        let now = now();
        let rows = sqlx::query(
            "SELECT payment_hash, created_at, retry_count, next_retry_at
             FROM newly_paid
             WHERE next_retry_at <= $1
             ORDER BY next_retry_at ASC",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let newly_paid = rows
            .into_iter()
            .map(|row| {
                Ok::<_, sqlx::Error>(NewlyPaid {
                    payment_hash: row.try_get(0)?,
                    created_at: row.try_get(1)?,
                    retry_count: row.try_get(2)?,
                    next_retry_at: row.try_get(3)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(newly_paid)
    }

    async fn update_newly_paid_retry(
        &self,
        payment_hash: &str,
        retry_count: i32,
        next_retry_at: i64,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "UPDATE newly_paid
             SET retry_count = $2, next_retry_at = $3
             WHERE payment_hash = $1",
        )
        .bind(payment_hash)
        .bind(retry_count)
        .bind(next_retry_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_newly_paid(&self, payment_hash: &str) -> Result<(), LnurlRepositoryError> {
        sqlx::query("DELETE FROM newly_paid WHERE payment_hash = $1")
            .bind(payment_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
