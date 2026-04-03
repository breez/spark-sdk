use lnurl_models::ListMetadataMetadata;
use sqlx::{Row, SqlitePool};

use crate::repository::{Invoice, LnurlSenderComment, PendingZapReceipt};
use crate::zap::Zap;
use crate::{
    repository::LnurlRepositoryError,
    time::{now, now_millis},
    user::User,
};

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
        .map(|row| {
            Ok::<_, sqlx::Error>(User {
                domain: domain.to_string(),
                pubkey: row.try_get(0)?,
                name: row.try_get(1)?,
                description: row.try_get(2)?,
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
            "SELECT pubkey, name, description
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
            })
        })
        .transpose()?;
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

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "REPLACE INTO zaps (payment_hash, zap_request, zap_event
            , user_pubkey, invoice_expiry, updated_at, is_user_nostr_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7)",
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
            "SELECT payment_hash, zap_request, zap_event
            , user_pubkey, invoice_expiry, updated_at, is_user_nostr_key
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
            "SELECT ph.payment_hash
             ,      sc.sender_comment
             ,      z.zap_request
             ,      z.zap_event
             ,      MAX(COALESCE(z.updated_at, 0), COALESCE(sc.updated_at, 0), COALESCE(i.updated_at, 0)) AS updated_at
             ,      i.preimage
             FROM (
                 SELECT payment_hash FROM invoices WHERE user_pubkey = $1 AND updated_at > $4
                 UNION
                 SELECT payment_hash FROM zaps WHERE user_pubkey = $1 AND updated_at > $4
                 UNION
                 SELECT payment_hash FROM sender_comments WHERE user_pubkey = $1 AND updated_at > $4
             ) ph
             LEFT JOIN invoices i ON ph.payment_hash = i.payment_hash
             LEFT JOIN zaps z ON ph.payment_hash = z.payment_hash
             LEFT JOIN sender_comments sc ON ph.payment_hash = sc.payment_hash
             ORDER BY updated_at ASC
             LIMIT $3 OFFSET $2",
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

    async fn filter_known_payment_hashes(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<String>, LnurlRepositoryError> {
        if payment_hashes.is_empty() {
            return Ok(vec![]);
        }

        let placeholders: Vec<String> = (1..=payment_hashes.len())
            .map(|i| format!("${i}"))
            .collect();
        let placeholders = placeholders.join(",");

        let query = format!(
            "SELECT payment_hash FROM invoices WHERE payment_hash IN ({placeholders})
             UNION
             SELECT payment_hash FROM zaps WHERE payment_hash IN ({placeholders})
             UNION
             SELECT payment_hash FROM sender_comments WHERE payment_hash IN ({placeholders})"
        );

        let mut q = sqlx::query_scalar::<_, String>(&query);
        // Bind three times (once per subquery in the UNION)
        for _ in 0..3 {
            for hash in payment_hashes {
                q = q.bind(hash);
            }
        }
        let known = q.fetch_all(&self.pool).await?;
        Ok(known)
    }

    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO invoices (payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at, domain, amount_received_sat)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT(payment_hash) DO UPDATE SET
                user_pubkey = excluded.user_pubkey,
                invoice = excluded.invoice,
                preimage = excluded.preimage,
                invoice_expiry = excluded.invoice_expiry,
                updated_at = excluded.updated_at,
                domain = excluded.domain,
                amount_received_sat = excluded.amount_received_sat",
        )
        .bind(&invoice.payment_hash)
        .bind(&invoice.user_pubkey)
        .bind(&invoice.invoice)
        .bind(&invoice.preimage)
        .bind(invoice.invoice_expiry)
        .bind(invoice.created_at)
        .bind(invoice.updated_at)
        .bind(&invoice.domain)
        .bind(invoice.amount_received_sat)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn upsert_invoices_paid(
        &self,
        invoices: &[Invoice],
    ) -> Result<Vec<String>, LnurlRepositoryError> {
        if invoices.is_empty() {
            return Ok(vec![]);
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        let mut affected = Vec::new();
        for invoice in invoices {
            let row: Option<(String,)> = sqlx::query_as(
                "INSERT INTO invoices (payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT(payment_hash) DO UPDATE SET
                    preimage = excluded.preimage,
                    updated_at = excluded.updated_at
                WHERE invoices.user_pubkey = excluded.user_pubkey AND invoices.preimage IS NULL
                RETURNING payment_hash",
            )
            .bind(&invoice.payment_hash)
            .bind(&invoice.user_pubkey)
            .bind(&invoice.invoice)
            .bind(&invoice.preimage)
            .bind(invoice.invoice_expiry)
            .bind(invoice.created_at)
            .bind(invoice.updated_at)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some((payment_hash,)) = row {
                affected.push(payment_hash);
            }
        }
        tx.commit()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        Ok(affected)
    }

    async fn get_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Invoice>, LnurlRepositoryError> {
        let maybe_invoice = sqlx::query(
            "SELECT payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at, domain, amount_received_sat
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
                domain: row.try_get(7)?,
                amount_received_sat: row.try_get(8)?,
            })
        })
        .transpose()?;
        Ok(maybe_invoice)
    }

    async fn get_zap_and_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<(Option<Zap>, Option<Invoice>), LnurlRepositoryError> {
        let row = sqlx::query(
            "SELECT z.payment_hash   AS z_payment_hash
             ,      z.zap_request    AS z_zap_request
             ,      z.zap_event      AS z_zap_event
             ,      z.user_pubkey    AS z_user_pubkey
             ,      z.invoice_expiry AS z_invoice_expiry
             ,      z.updated_at     AS z_updated_at
             ,      z.is_user_nostr_key AS z_is_user_nostr_key
             ,      i.payment_hash   AS i_payment_hash
             ,      i.user_pubkey    AS i_user_pubkey
             ,      i.invoice        AS i_invoice
             ,      i.preimage       AS i_preimage
             ,      i.invoice_expiry AS i_invoice_expiry
             ,      i.created_at     AS i_created_at
             ,      i.updated_at     AS i_updated_at
             ,      i.domain         AS i_domain
             ,      i.amount_received_sat AS i_amount_received_sat
             FROM (SELECT $1 AS payment_hash) ph
             LEFT JOIN zaps z ON z.payment_hash = ph.payment_hash
             LEFT JOIN invoices i ON i.payment_hash = ph.payment_hash",
        )
        .bind(payment_hash)
        .fetch_one(&self.pool)
        .await?;

        let zap = row
            .try_get::<Option<String>, _>("z_payment_hash")?
            .map(|ph| {
                Ok::<_, sqlx::Error>(Zap {
                    payment_hash: ph,
                    zap_request: row.try_get("z_zap_request")?,
                    zap_event: row.try_get("z_zap_event")?,
                    user_pubkey: row.try_get("z_user_pubkey")?,
                    invoice_expiry: row.try_get("z_invoice_expiry")?,
                    updated_at: row.try_get("z_updated_at")?,
                    is_user_nostr_key: row.try_get("z_is_user_nostr_key")?,
                })
            })
            .transpose()?;

        let invoice = row
            .try_get::<Option<String>, _>("i_payment_hash")?
            .map(|ph| {
                Ok::<_, sqlx::Error>(Invoice {
                    payment_hash: ph,
                    user_pubkey: row.try_get("i_user_pubkey")?,
                    invoice: row.try_get("i_invoice")?,
                    preimage: row.try_get("i_preimage")?,
                    invoice_expiry: row.try_get("i_invoice_expiry")?,
                    created_at: row.try_get("i_created_at")?,
                    updated_at: row.try_get("i_updated_at")?,
                    domain: row.try_get("i_domain")?,
                    amount_received_sat: row.try_get("i_amount_received_sat")?,
                })
            })
            .transpose()?;

        Ok((zap, invoice))
    }
    async fn insert_pending_zap_receipt(
        &self,
        pending: &PendingZapReceipt,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO pending_zap_receipts (payment_hash, created_at, retry_count, next_retry_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(payment_hash) DO NOTHING",
        )
        .bind(&pending.payment_hash)
        .bind(pending.created_at)
        .bind(pending.retry_count)
        .bind(pending.next_retry_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_pending_zap_receipt_batch(
        &self,
        pending: &[PendingZapReceipt],
    ) -> Result<(), LnurlRepositoryError> {
        if pending.is_empty() {
            return Ok(());
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        for item in pending {
            sqlx::query(
                "INSERT INTO pending_zap_receipts (payment_hash, created_at, retry_count, next_retry_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT(payment_hash) DO NOTHING",
            )
            .bind(&item.payment_hash)
            .bind(item.created_at)
            .bind(item.retry_count)
            .bind(item.next_retry_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        Ok(())
    }

    async fn take_pending_zap_receipts(
        &self,
        limit: u32,
    ) -> Result<Vec<PendingZapReceipt>, LnurlRepositoryError> {
        let now = now_millis();
        let stale_threshold = now.saturating_sub(300_000); // 5 minutes
        let rows = sqlx::query(
            "UPDATE pending_zap_receipts
             SET claimed_at = $2
             WHERE payment_hash IN (
                 SELECT payment_hash FROM pending_zap_receipts
                 WHERE next_retry_at <= $1
                   AND COALESCE(claimed_at, 0) < $3
                 ORDER BY next_retry_at ASC
                 LIMIT $4
             )
             RETURNING payment_hash, created_at, retry_count, next_retry_at",
        )
        .bind(now)
        .bind(now)
        .bind(stale_threshold)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let pending = rows
            .into_iter()
            .map(|row| {
                Ok::<_, sqlx::Error>(PendingZapReceipt {
                    payment_hash: row.try_get(0)?,
                    created_at: row.try_get(1)?,
                    retry_count: row.try_get(2)?,
                    next_retry_at: row.try_get(3)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(pending)
    }

    async fn update_pending_zap_receipt_retry(
        &self,
        payment_hash: &str,
        retry_count: i32,
        next_retry_at: i64,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "UPDATE pending_zap_receipts
             SET retry_count = $2, next_retry_at = $3, claimed_at = NULL
             WHERE payment_hash = $1",
        )
        .bind(payment_hash)
        .bind(retry_count)
        .bind(next_retry_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_pending_zap_receipt(
        &self,
        payment_hash: &str,
    ) -> Result<(), LnurlRepositoryError> {
        sqlx::query("DELETE FROM pending_zap_receipts WHERE payment_hash = $1")
            .bind(payment_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_or_create_setting(
        &self,
        key: &str,
        default_value: &str,
    ) -> Result<String, LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ($1, $2)
             ON CONFLICT(key) DO NOTHING",
        )
        .bind(key)
        .bind(default_value)
        .execute(&self.pool)
        .await?;

        let value: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_one(&self.pool)
            .await?;
        Ok(value)
    }
}
