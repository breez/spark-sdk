use lnurl_models::ListMetadataMetadata;
use sqlx::{PgPool, Row};

use crate::repository::{Invoice, LnurlSenderComment, PendingZapReceipt, WebhookPayloadData};
use crate::webhooks::repository::{
    NewWebhookDelivery, WebhookConfig, WebhookDelivery, WebhookRepositoryError,
};
use crate::zap::Zap;
use crate::{
    repository::LnurlRepositoryError,
    time::{now, now_millis},
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
            "INSERT INTO users (domain, pubkey, name, description, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT(domain, pubkey) DO UPDATE
             SET name = excluded.name
             ,   description = excluded.description
             ,   updated_at = excluded.updated_at",
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

    async fn transfer_username(
        &self,
        domain: &str,
        from_pubkey: &str,
        to_pubkey: &str,
        username: &str,
        description: &str,
    ) -> Result<(), LnurlRepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;

        let source_name: Option<(String,)> =
            sqlx::query_as("SELECT name FROM users WHERE domain = $1 AND pubkey = $2 FOR UPDATE")
                .bind(domain)
                .bind(from_pubkey)
                .fetch_optional(&mut *tx)
                .await?;
        match source_name {
            Some((name,)) if name == username => {}
            _ => return Err(LnurlRepositoryError::SourceNotOwner),
        }

        sqlx::query("DELETE FROM users WHERE domain = $1 AND pubkey = $2")
            .bind(domain)
            .bind(from_pubkey)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO users (domain, pubkey, name, description, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT(domain, pubkey) DO UPDATE
             SET name = excluded.name
             ,   description = excluded.description
             ,   updated_at = excluded.updated_at",
        )
        .bind(domain)
        .bind(to_pubkey)
        .bind(username)
        .bind(description)
        .bind(now())
        .execute(&mut *tx)
        .await?;

        tx.commit()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
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
             ,      GREATEST(COALESCE(z.updated_at, 0), COALESCE(sc.updated_at, 0), COALESCE(i.updated_at, 0)) AS updated_at
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

    async fn filter_known_payment_hashes(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<String>, LnurlRepositoryError> {
        if payment_hashes.is_empty() {
            return Ok(vec![]);
        }

        let known: Vec<String> = sqlx::query_scalar(
            "SELECT payment_hash FROM invoices WHERE payment_hash = ANY($1)
             UNION
             SELECT payment_hash FROM zaps WHERE payment_hash = ANY($1)
             UNION
             SELECT payment_hash FROM sender_comments WHERE payment_hash = ANY($1)",
        )
        .bind(payment_hashes)
        .fetch_all(&self.pool)
        .await?;
        Ok(known)
    }

    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError> {
        sqlx::query(
            "INSERT INTO invoices (payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at, domain, amount_received_sat)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT(payment_hash) DO UPDATE
             SET user_pubkey = excluded.user_pubkey
             ,   invoice = excluded.invoice
             ,   preimage = excluded.preimage
             ,   invoice_expiry = excluded.invoice_expiry
             ,   updated_at = excluded.updated_at
             ,   domain = excluded.domain
             ,   amount_received_sat = excluded.amount_received_sat",
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
        let payment_hashes: Vec<&str> = invoices.iter().map(|i| i.payment_hash.as_str()).collect();
        let user_pubkeys: Vec<&str> = invoices.iter().map(|i| i.user_pubkey.as_str()).collect();
        let invoice_strs: Vec<&str> = invoices.iter().map(|i| i.invoice.as_str()).collect();
        let preimages: Vec<Option<&str>> = invoices.iter().map(|i| i.preimage.as_deref()).collect();
        let invoice_expiries: Vec<i64> = invoices.iter().map(|i| i.invoice_expiry).collect();
        let created_ats: Vec<i64> = invoices.iter().map(|i| i.created_at).collect();
        let updated_ats: Vec<i64> = invoices.iter().map(|i| i.updated_at).collect();

        let rows = sqlx::query(
            "INSERT INTO invoices (payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at)
             SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[], $5::bigint[], $6::bigint[], $7::bigint[])
             ON CONFLICT(payment_hash) DO UPDATE
             SET preimage = excluded.preimage
             ,   updated_at = excluded.updated_at
             WHERE invoices.user_pubkey = excluded.user_pubkey AND invoices.preimage IS NULL
             RETURNING payment_hash",
        )
        .bind(&payment_hashes)
        .bind(&user_pubkeys)
        .bind(&invoice_strs)
        .bind(&preimages)
        .bind(&invoice_expiries)
        .bind(&created_ats)
        .bind(&updated_ats)
        .fetch_all(&self.pool)
        .await?;

        let affected = rows
            .into_iter()
            .map(|row| row.try_get(0))
            .collect::<Result<Vec<String>, sqlx::Error>>()?;
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
             FROM (SELECT $1::text AS payment_hash) ph
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
        let payment_hashes: Vec<&str> = pending.iter().map(|n| n.payment_hash.as_str()).collect();
        let created_ats: Vec<i64> = pending.iter().map(|n| n.created_at).collect();
        let retry_counts: Vec<i32> = pending.iter().map(|n| n.retry_count).collect();
        let next_retry_ats: Vec<i64> = pending.iter().map(|n| n.next_retry_at).collect();

        sqlx::query(
            "INSERT INTO pending_zap_receipts (payment_hash, created_at, retry_count, next_retry_at)
             SELECT * FROM UNNEST($1::text[], $2::bigint[], $3::int[], $4::bigint[])
             ON CONFLICT(payment_hash) DO NOTHING",
        )
        .bind(&payment_hashes)
        .bind(&created_ats)
        .bind(&retry_counts)
        .bind(&next_retry_ats)
        .execute(&self.pool)
        .await?;
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
                 FOR UPDATE SKIP LOCKED
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
        let value: String = sqlx::query_scalar(
            "INSERT INTO settings (key, value) VALUES ($1, $2)
             ON CONFLICT(key) DO UPDATE SET value = settings.value
             RETURNING value",
        )
        .bind(key)
        .bind(default_value)
        .fetch_one(&self.pool)
        .await?;
        Ok(value)
    }

    async fn get_webhook_payloads(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<WebhookPayloadData>, LnurlRepositoryError> {
        if payment_hashes.is_empty() {
            return Ok(vec![]);
        }
        let hashes: Vec<&str> = payment_hashes.iter().map(String::as_str).collect();
        let rows = sqlx::query(
            "SELECT i.payment_hash, i.user_pubkey, i.invoice, i.preimage, i.amount_received_sat,
                    u.name, u.domain,
                    sc.sender_comment,
                    i.domain
             FROM invoices i
             LEFT JOIN users u ON u.pubkey = i.user_pubkey AND u.domain = i.domain
             LEFT JOIN sender_comments sc ON sc.payment_hash = i.payment_hash
             WHERE i.payment_hash = ANY($1)
               AND i.domain IS NOT NULL
               AND i.preimage IS NOT NULL",
        )
        .bind(&hashes)
        .fetch_all(&self.pool)
        .await?;
        let results = rows
            .into_iter()
            .map(|row| {
                let name: Option<String> = row.try_get(5)?;
                let user_domain: Option<String> = row.try_get(6)?;
                let lightning_address = match (name, user_domain) {
                    (Some(n), Some(d)) => Some(format!("{n}@{d}")),
                    _ => None,
                };
                Ok::<_, sqlx::Error>(WebhookPayloadData {
                    payment_hash: row.try_get(0)?,
                    user_pubkey: row.try_get(1)?,
                    invoice: row.try_get(2)?,
                    preimage: row.try_get(3)?,
                    amount_received_sat: row.try_get(4)?,
                    lightning_address,
                    sender_comment: row.try_get(7)?,
                    domain: row.try_get(8)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }
}

#[async_trait::async_trait]
impl crate::webhooks::WebhookRepository for LnurlRepository {
    async fn insert_webhook_deliveries(
        &self,
        deliveries: &[NewWebhookDelivery],
    ) -> Result<(), WebhookRepositoryError> {
        if deliveries.is_empty() {
            return Ok(());
        }
        let now = now_millis();
        let identifiers: Vec<&str> = deliveries.iter().map(|d| d.identifier.as_str()).collect();
        let domains: Vec<&str> = deliveries.iter().map(|d| d.domain.as_str()).collect();
        let payloads: Vec<&str> = deliveries.iter().map(|d| d.payload.as_str()).collect();
        let created_ats: Vec<i64> = vec![now; deliveries.len()];

        sqlx::query(
            "INSERT INTO webhook_deliveries (identifier, domain, payload, created_at, next_retry_at)
             SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::bigint[], $4::bigint[])
             ON CONFLICT (identifier, domain) DO NOTHING",
        )
        .bind(&identifiers)
        .bind(&domains)
        .bind(&payloads)
        .bind(&created_ats)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn take_pending_webhook_deliveries(
        &self,
    ) -> Result<Vec<WebhookDelivery>, WebhookRepositoryError> {
        let now = now_millis();
        let stale_threshold = now.saturating_sub(300_000); // 5 minutes
        let rows = sqlx::query(
            "UPDATE webhook_deliveries
             SET claimed_at = $2
             WHERE id IN (
                 SELECT d.id
                 FROM (
                     SELECT DISTINCT domain
                     FROM webhook_deliveries
                     WHERE next_retry_at <= $1
                       AND succeeded_at IS NULL
                       AND COALESCE(claimed_at, 0) < $3
                 ) domains
                 CROSS JOIN LATERAL (
                     SELECT id
                     FROM webhook_deliveries
                     WHERE domain = domains.domain
                       AND next_retry_at <= $1
                       AND succeeded_at IS NULL
                       AND COALESCE(claimed_at, 0) < $3
                     ORDER BY next_retry_at ASC
                     FOR UPDATE SKIP LOCKED
                     LIMIT 1
                 ) d
             )
             RETURNING id, identifier, domain, url, payload, created_at, retry_count, next_retry_at",
        )
        .bind(now)
        .bind(now)
        .bind(stale_threshold)
        .fetch_all(&self.pool)
        .await?;
        let deliveries = rows
            .into_iter()
            .map(|row| {
                Ok::<_, sqlx::Error>(WebhookDelivery {
                    id: row.try_get(0)?,
                    identifier: row.try_get(1)?,
                    domain: row.try_get(2)?,
                    url: row.try_get(3)?,
                    payload: row.try_get(4)?,
                    created_at: row.try_get(5)?,
                    retry_count: row.try_get(6)?,
                    next_retry_at: row.try_get(7)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(deliveries)
    }

    async fn update_webhook_delivery_success(
        &self,
        id: i64,
        succeeded_at: i64,
        url: &str,
    ) -> Result<(), WebhookRepositoryError> {
        sqlx::query("UPDATE webhook_deliveries SET succeeded_at = $2, url = $3 WHERE id = $1")
            .bind(id)
            .bind(succeeded_at)
            .bind(url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_webhook_delivery_failure(
        &self,
        id: i64,
        retry_count: i32,
        next_retry_at: i64,
        status_code: Option<i32>,
        body: Option<&str>,
        url: &str,
    ) -> Result<(), WebhookRepositoryError> {
        sqlx::query(
            "UPDATE webhook_deliveries
             SET retry_count = $2, next_retry_at = $3, claimed_at = NULL,
                 last_error_status_code = $4, last_error_body = $5, url = $6
             WHERE id = $1",
        )
        .bind(id)
        .bind(retry_count)
        .bind(next_retry_at)
        .bind(status_code)
        .bind(body)
        .bind(url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn unclaim_webhook_deliveries(&self, ids: &[i64]) -> Result<(), WebhookRepositoryError> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query("UPDATE webhook_deliveries SET claimed_at = NULL WHERE id = ANY($1)")
            .bind(ids)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_webhook_deliveries_older_than(
        &self,
        before: i64,
    ) -> Result<u64, WebhookRepositoryError> {
        let result = sqlx::query("DELETE FROM webhook_deliveries WHERE created_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn delete_webhook_delivery(&self, id: i64) -> Result<(), WebhookRepositoryError> {
        sqlx::query("DELETE FROM webhook_deliveries WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn park_webhook_delivery(&self, id: i64) -> Result<(), WebhookRepositoryError> {
        sqlx::query(
            "UPDATE webhook_deliveries SET next_retry_at = $2, claimed_at = NULL WHERE id = $1",
        )
        .bind(id)
        .bind(i64::MAX)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_webhook_configs(&self) -> Result<Vec<WebhookConfig>, WebhookRepositoryError> {
        let rows = sqlx::query("SELECT domain, url, webhook_secret FROM domain_webhooks")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(WebhookConfig {
                    domain: row.try_get(0)?,
                    url: row.try_get(1)?,
                    secret: row.try_get(2)?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(|e| WebhookRepositoryError::General(e.into()))
    }
}
