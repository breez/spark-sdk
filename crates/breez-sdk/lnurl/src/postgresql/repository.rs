use lnurl_models::ListMetadataMetadata;
use spark_postgres::deadpool_postgres::Pool;

use crate::repository::{
    DomainConfig, Invoice, LnurlSenderComment, PendingZapReceipt, WebhookPayloadData,
};
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
    pool: Pool,
}

impl LnurlRepository {
    pub fn new(pool: Pool) -> Self {
        LnurlRepository { pool }
    }
}

/// `users.updated_at` is an `INTEGER` column, so the timestamp is narrowed
/// before binding. Saturating keeps writes working past 2038 rather than
/// failing them, at the cost of a frozen timestamp.
///
/// TODO: widen the column to BIGINT, matching every other timestamp in the
/// schema, and bind `now()` directly.
fn now_secs_i32() -> i32 {
    i32::try_from(now()).unwrap_or(i32::MAX)
}

#[async_trait::async_trait]
impl crate::repository::LnurlRepository for LnurlRepository {
    async fn delete_user(
        &self,
        domain: &str,
        pubkey: &str,
        name: &str,
    ) -> Result<bool, LnurlRepositoryError> {
        let client = self.pool.get().await?;
        let rows_affected = client
            .execute(
                "DELETE FROM users WHERE domain = $1 AND pubkey = $2 AND name = $3",
                &[&domain, &pubkey, &name],
            )
            .await?;
        Ok(rows_affected > 0)
    }

    async fn get_user_by_name(
        &self,
        domain: &str,
        name: &str,
    ) -> Result<Option<User>, LnurlRepositoryError> {
        let client = self.pool.get().await?;
        let maybe_user = client
            .query_opt(
                "SELECT pubkey, name, description
                 FROM users
                 WHERE domain = $1 AND name = $2",
                &[&domain, &name],
            )
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
        let client = self.pool.get().await?;
        let maybe_user = client
            .query_opt(
                "SELECT pubkey, name, description
                 FROM users
                 WHERE domain = $1 AND pubkey = $2",
                &[&domain, &pubkey],
            )
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
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO users (domain, pubkey, name, description, updated_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT(domain, pubkey) DO UPDATE
                 SET name = excluded.name
                 ,   description = excluded.description
                 ,   updated_at = excluded.updated_at",
                &[
                    &user.domain,
                    &user.pubkey,
                    &user.name,
                    &user.description,
                    &now_secs_i32(),
                ],
            )
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
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        let source_name = tx
            .query_opt(
                "DELETE FROM users WHERE domain = $1 AND pubkey = $2 RETURNING name",
                &[&domain, &from_pubkey],
            )
            .await?
            .map(|row| row.get::<_, String>(0));
        match source_name {
            Some(name) if name == username => {}
            // Source pubkey doesn't currently own this username. The tx is
            // rolled back on drop, so the speculative DELETE is undone.
            _ => return Err(LnurlRepositoryError::SourceNotOwner),
        }

        tx.execute(
            "INSERT INTO users (domain, pubkey, name, description, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT(domain, pubkey) DO UPDATE
             SET name = excluded.name
             ,   description = excluded.description
             ,   updated_at = excluded.updated_at",
            &[
                &domain,
                &to_pubkey,
                &username,
                &description,
                &now_secs_i32(),
            ],
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
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
                &[
                    &zap.payment_hash,
                    &zap.zap_request,
                    &zap.zap_event,
                    &zap.user_pubkey,
                    &zap.invoice_expiry,
                    &zap.updated_at,
                    &zap.is_user_nostr_key,
                ],
            )
            .await?;
        Ok(())
    }

    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO sender_comments (payment_hash, user_pubkey, sender_comment, updated_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT(payment_hash) DO UPDATE
                 SET user_pubkey = excluded.user_pubkey
                 ,   sender_comment = excluded.sender_comment
                 ,   updated_at = excluded.updated_at",
                &[
                    &comment.payment_hash,
                    &comment.user_pubkey,
                    &comment.comment,
                    &comment.updated_at,
                ],
            )
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
        let offset = i64::from(offset);
        let limit = i64::from(limit);
        let client = self.pool.get().await?;
        let rows = client
            .query(
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
                &[&pubkey, &offset, &limit, &updated_after],
            )
            .await?;
        let metadata = rows
            .into_iter()
            .map(|row| ListMetadataMetadata {
                payment_hash: row.get(0),
                sender_comment: row.get(1),
                nostr_zap_request: row.get(2),
                nostr_zap_receipt: row.get(3),
                updated_at: row.get(4),
                preimage: row.get(5),
            })
            .collect();
        Ok(metadata)
    }

    async fn list_domains(&self) -> Result<Vec<DomainConfig>, LnurlRepositoryError> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT d.domain, a.api_key, a.jwt \
                 FROM allowed_domains d \
                 LEFT JOIN domain_attribution a ON a.domain = d.domain",
                &[],
            )
            .await?;

        let domains = rows
            .into_iter()
            .map(|row| DomainConfig {
                domain: row.get(0),
                api_key: row.get(1),
                jwt: row.get(2),
            })
            .collect();

        Ok(domains)
    }

    async fn add_domain(&self, domain: &str) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO allowed_domains (domain)
                 VALUES ($1)
                 ON CONFLICT(domain) DO NOTHING",
                &[&domain],
            )
            .await?;
        Ok(())
    }

    async fn set_domain_jwt(&self, domain: &str, jwt: &str) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE domain_attribution SET jwt = $2 WHERE domain = $1",
                &[&domain, &jwt],
            )
            .await?;
        Ok(())
    }

    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
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
                &[
                    &invoice.payment_hash,
                    &invoice.user_pubkey,
                    &invoice.invoice,
                    &invoice.preimage,
                    &invoice.invoice_expiry,
                    &invoice.created_at,
                    &invoice.updated_at,
                    &invoice.domain,
                    &invoice.amount_received_sat,
                ],
            )
            .await?;
        Ok(())
    }

    async fn get_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Invoice>, LnurlRepositoryError> {
        let client = self.pool.get().await?;
        let maybe_invoice = client
            .query_opt(
                "SELECT payment_hash, user_pubkey, invoice, preimage, invoice_expiry, created_at, updated_at, domain, amount_received_sat
                 FROM invoices
                 WHERE payment_hash = $1",
                &[&payment_hash],
            )
            .await?
            .map(|row| Invoice {
                payment_hash: row.get(0),
                user_pubkey: row.get(1),
                invoice: row.get(2),
                preimage: row.get(3),
                invoice_expiry: row.get(4),
                created_at: row.get(5),
                updated_at: row.get(6),
                domain: row.get(7),
                amount_received_sat: row.get(8),
            });
        Ok(maybe_invoice)
    }

    async fn get_zap_and_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<(Option<Zap>, Option<Invoice>), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
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
                &[&payment_hash],
            )
            .await?;

        let zap = row
            .get::<_, Option<String>>("z_payment_hash")
            .map(|ph| Zap {
                payment_hash: ph,
                zap_request: row.get("z_zap_request"),
                zap_event: row.get("z_zap_event"),
                user_pubkey: row.get("z_user_pubkey"),
                invoice_expiry: row.get("z_invoice_expiry"),
                updated_at: row.get("z_updated_at"),
                is_user_nostr_key: row.get("z_is_user_nostr_key"),
            });

        let invoice = row
            .get::<_, Option<String>>("i_payment_hash")
            .map(|ph| Invoice {
                payment_hash: ph,
                user_pubkey: row.get("i_user_pubkey"),
                invoice: row.get("i_invoice"),
                preimage: row.get("i_preimage"),
                invoice_expiry: row.get("i_invoice_expiry"),
                created_at: row.get("i_created_at"),
                updated_at: row.get("i_updated_at"),
                domain: row.get("i_domain"),
                amount_received_sat: row.get("i_amount_received_sat"),
            });

        Ok((zap, invoice))
    }
    async fn insert_pending_zap_receipt(
        &self,
        pending: &PendingZapReceipt,
    ) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO pending_zap_receipts (payment_hash, created_at, retry_count, next_retry_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT(payment_hash) DO NOTHING",
                &[
                    &pending.payment_hash,
                    &pending.created_at,
                    &pending.retry_count,
                    &pending.next_retry_at,
                ],
            )
            .await?;
        Ok(())
    }

    async fn take_pending_zap_receipts(
        &self,
        limit: u32,
    ) -> Result<Vec<PendingZapReceipt>, LnurlRepositoryError> {
        let now = now_millis();
        let stale_threshold = now.saturating_sub(300_000); // 5 minutes
        let limit = i64::from(limit);
        let client = self.pool.get().await?;
        let rows = client
            .query(
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
                &[&now, &now, &stale_threshold, &limit],
            )
            .await?;
        let pending = rows
            .into_iter()
            .map(|row| PendingZapReceipt {
                payment_hash: row.get(0),
                created_at: row.get(1),
                retry_count: row.get(2),
                next_retry_at: row.get(3),
            })
            .collect();
        Ok(pending)
    }

    async fn update_pending_zap_receipt_retry(
        &self,
        payment_hash: &str,
        retry_count: i32,
        next_retry_at: i64,
    ) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE pending_zap_receipts
                 SET retry_count = $2, next_retry_at = $3, claimed_at = NULL
                 WHERE payment_hash = $1",
                &[&payment_hash, &retry_count, &next_retry_at],
            )
            .await?;
        Ok(())
    }

    async fn delete_pending_zap_receipt(
        &self,
        payment_hash: &str,
    ) -> Result<(), LnurlRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "DELETE FROM pending_zap_receipts WHERE payment_hash = $1",
                &[&payment_hash],
            )
            .await?;
        Ok(())
    }

    async fn get_or_create_setting(
        &self,
        key: &str,
        default_value: &str,
    ) -> Result<String, LnurlRepositoryError> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "INSERT INTO settings (key, value) VALUES ($1, $2)
                 ON CONFLICT(key) DO UPDATE SET value = settings.value
                 RETURNING value",
                &[&key, &default_value],
            )
            .await?;
        Ok(row.get(0))
    }

    async fn get_webhook_payloads(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<WebhookPayloadData>, LnurlRepositoryError> {
        if payment_hashes.is_empty() {
            return Ok(vec![]);
        }
        let client = self.pool.get().await?;
        let rows = client
            .query(
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
                &[&payment_hashes],
            )
            .await?;
        let results = rows
            .into_iter()
            .map(|row| {
                let name: Option<String> = row.get(5);
                let user_domain: Option<String> = row.get(6);
                let lightning_address = match (name, user_domain) {
                    (Some(n), Some(d)) => Some(format!("{n}@{d}")),
                    _ => None,
                };
                WebhookPayloadData {
                    payment_hash: row.get(0),
                    user_pubkey: row.get(1),
                    invoice: row.get(2),
                    preimage: row.get(3),
                    amount_received_sat: row.get(4),
                    lightning_address,
                    sender_comment: row.get(7),
                    domain: row.get(8),
                }
            })
            .collect();
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

        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO webhook_deliveries (identifier, domain, payload, created_at, next_retry_at)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::bigint[], $4::bigint[])
                 ON CONFLICT (identifier, domain) DO NOTHING",
                &[&identifiers, &domains, &payloads, &created_ats],
            )
            .await?;
        Ok(())
    }

    async fn take_pending_webhook_deliveries(
        &self,
    ) -> Result<Vec<WebhookDelivery>, WebhookRepositoryError> {
        let now = now_millis();
        let stale_threshold = now.saturating_sub(300_000); // 5 minutes
        let client = self.pool.get().await?;
        let rows = client
            .query(
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
                &[&now, &now, &stale_threshold],
            )
            .await?;
        let deliveries = rows
            .into_iter()
            .map(|row| WebhookDelivery {
                id: row.get(0),
                identifier: row.get(1),
                domain: row.get(2),
                url: row.get(3),
                payload: row.get(4),
                created_at: row.get(5),
                retry_count: row.get(6),
                next_retry_at: row.get(7),
            })
            .collect();
        Ok(deliveries)
    }

    async fn update_webhook_delivery_success(
        &self,
        id: i64,
        succeeded_at: i64,
        url: &str,
    ) -> Result<(), WebhookRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE webhook_deliveries SET succeeded_at = $2, url = $3 WHERE id = $1",
                &[&id, &succeeded_at, &url],
            )
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
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE webhook_deliveries
                 SET retry_count = $2, next_retry_at = $3, claimed_at = NULL,
                     last_error_status_code = $4, last_error_body = $5, url = $6
                 WHERE id = $1",
                &[&id, &retry_count, &next_retry_at, &status_code, &body, &url],
            )
            .await?;
        Ok(())
    }

    async fn unclaim_webhook_deliveries(&self, ids: &[i64]) -> Result<(), WebhookRepositoryError> {
        if ids.is_empty() {
            return Ok(());
        }
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE webhook_deliveries SET claimed_at = NULL WHERE id = ANY($1)",
                &[&ids],
            )
            .await?;
        Ok(())
    }

    async fn delete_webhook_deliveries_older_than(
        &self,
        before: i64,
    ) -> Result<u64, WebhookRepositoryError> {
        let client = self.pool.get().await?;
        let rows_affected = client
            .execute(
                "DELETE FROM webhook_deliveries WHERE created_at < $1",
                &[&before],
            )
            .await?;
        Ok(rows_affected)
    }

    async fn delete_webhook_delivery(&self, id: i64) -> Result<(), WebhookRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute("DELETE FROM webhook_deliveries WHERE id = $1", &[&id])
            .await?;
        Ok(())
    }

    async fn park_webhook_delivery(&self, id: i64) -> Result<(), WebhookRepositoryError> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE webhook_deliveries SET next_retry_at = $2, claimed_at = NULL WHERE id = $1",
                &[&id, &i64::MAX],
            )
            .await?;
        Ok(())
    }

    async fn list_webhook_configs(&self) -> Result<Vec<WebhookConfig>, WebhookRepositoryError> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT domain, url, webhook_secret FROM domain_webhooks",
                &[],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| WebhookConfig {
                domain: row.get(0),
                url: row.get(1),
                secret: row.get(2),
            })
            .collect())
    }
}

#[cfg(test)]
mod postgres_tests {
    use spark_postgres::deadpool_postgres::Pool;

    use crate::repository::shared_tests;
    use crate::test_support::test_pool;

    /// Seed `a.com` with an api key the way admins do: allowlist it, then set its
    /// key in the attribution table.
    async fn seed_domain_with_api_key(pool: &Pool) {
        let client = pool.get().await.unwrap();
        client
            .execute("INSERT INTO allowed_domains (domain) VALUES ('a.com')", &[])
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO domain_attribution (domain, api_key) VALUES ('a.com', 'key-a')",
                &[],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_domains_surfaces_api_keys() {
        let (_pg, pool) = test_pool().await;
        seed_domain_with_api_key(&pool).await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::list_domains_surfaces_api_keys(&db).await;
    }

    #[tokio::test]
    async fn set_domain_jwt_round_trips() {
        let (_pg, pool) = test_pool().await;
        seed_domain_with_api_key(&pool).await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::set_domain_jwt_round_trips(&db).await;
    }

    #[tokio::test]
    async fn registering_taken_name_with_other_pubkey_is_rejected() {
        let (_pg, pool) = test_pool().await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::registering_taken_name_with_other_pubkey_is_rejected(&db).await;
    }

    #[tokio::test]
    async fn deleting_a_name_the_pubkey_no_longer_holds_is_a_no_op() {
        let (_pg, pool) = test_pool().await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::deleting_a_name_the_pubkey_no_longer_holds_is_a_no_op(&db).await;
    }
}
