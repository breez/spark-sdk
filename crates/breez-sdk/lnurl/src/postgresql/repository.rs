use bitcoin::hashes::{Hash, HashEngine, sha256};
use lnurl_models::ListMetadataMetadata;
use sqlx::{PgPool, Row};

use crate::repository::{
    DomainConfig, Invoice, LnurlSenderComment, NameStatus, PendingZapReceipt, RegistrationLimit,
    TransferRequest, WebhookPayloadData,
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
    pool: PgPool,
}

impl LnurlRepository {
    pub fn new(pool: PgPool) -> Self {
        LnurlRepository { pool }
    }
}

/// Record `statement_hash` as acted on, returning whether this call recorded it.
///
/// Runs on any executor, so a route that has to record inside its own
/// transaction shares this with one that records on its own.
async fn claim_statement<'e, E>(
    executor: E,
    statement_hash: &[u8],
    route: &str,
    expires_at: i64,
) -> Result<bool, LnurlRepositoryError>
where
    E: sqlx::PgExecutor<'e>,
{
    // DO NOTHING rather than DO UPDATE, so the first claim survives. A returned
    // row therefore means this call inserted it.
    let inserted: Option<(Vec<u8>,)> = sqlx::query_as(
        "INSERT INTO used_signed_messages
             (statement_hash, route, expires_at, created_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (statement_hash) DO NOTHING
         RETURNING statement_hash",
    )
    .bind(statement_hash)
    .bind(route)
    .bind(expires_at)
    .bind(now())
    .fetch_optional(executor)
    .await?;

    Ok(inserted.is_some())
}

/// Hold `name` for `pubkey`, so no one else may register it while the hold
/// stands.
///
/// The hold stands for good: `reclaimable_from` stays NULL until a policy that
/// lets holds lapse (a per-partner cooldown, say) writes one.
///
/// The last release wins, so a name reserved for a pubkey that took it back and
/// gave it up again is held for whoever gave it up last.
async fn reserve_name<'e, E>(
    executor: E,
    domain: &str,
    name: &str,
    pubkey: &str,
) -> Result<(), LnurlRepositoryError>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query(
        "INSERT INTO released_names (domain, name, pubkey, released_at, reclaimable_from)
         VALUES ($1, $2, $3, $4, NULL)
         ON CONFLICT (domain, name) DO UPDATE
         SET pubkey = excluded.pubkey
         ,   released_at = excluded.released_at
         ,   reclaimable_from = excluded.reclaimable_from",
    )
    .bind(domain)
    .bind(name)
    .bind(pubkey)
    .bind(now())
    .execute(executor)
    .await?;
    Ok(())
}

/// Whether `name` is held for a pubkey other than `pubkey`.
async fn reserved_for_other<'e, E>(
    executor: E,
    domain: &str,
    name: &str,
    pubkey: &str,
) -> Result<bool, LnurlRepositoryError>
where
    E: sqlx::PgExecutor<'e>,
{
    let held: Option<(String, Option<i64>)> = sqlx::query_as(
        "SELECT pubkey, reclaimable_from FROM released_names WHERE domain = $1 AND name = $2",
    )
    .bind(domain)
    .bind(name)
    .fetch_optional(executor)
    .await?;

    Ok(match held {
        Some((holder, reclaimable_from)) => {
            holder != pubkey && reclaimable_from.is_none_or(|from| now() < from)
        }
        None => false,
    })
}

/// The name `pubkey` holds in `domain`. Callers hold that registration's lock,
/// so the row cannot pick up a different name between this read and the write
/// that replaces it.
async fn name_of<'e, E>(
    executor: E,
    domain: &str,
    pubkey: &str,
) -> Result<Option<String>, LnurlRepositoryError>
where
    E: sqlx::PgExecutor<'e>,
{
    let held: Option<(String,)> =
        sqlx::query_as("SELECT name FROM users WHERE domain = $1 AND pubkey = $2")
            .bind(domain)
            .bind(pubkey)
            .fetch_optional(executor)
            .await?;
    Ok(held.map(|(name,)| name))
}

/// Log a registration that changed the held name and enforce `limit`, all
/// inside the caller's transaction: a refusal rolls back the registration and
/// the log row together. The caller holds the registration lock for this
/// pubkey, so the count cannot race a concurrent registration.
async fn record_registration(
    tx: &mut sqlx::PgConnection,
    domain: &str,
    pubkey: &str,
    limit: RegistrationLimit,
) -> Result<(), LnurlRepositoryError> {
    sqlx::query(
        "INSERT INTO address_registrations (domain, pubkey, created_at) VALUES ($1, $2, $3)",
    )
    .bind(domain)
    .bind(pubkey)
    .bind(now())
    .execute(&mut *tx)
    .await?;

    let cutoff = now().saturating_sub(i64::try_from(limit.window_secs).unwrap_or(i64::MAX));
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM address_registrations
         WHERE domain = $1 AND pubkey = $2 AND created_at > $3",
    )
    .bind(domain)
    .bind(pubkey)
    .bind(cutoff)
    .fetch_one(&mut *tx)
    .await?;

    if count > i64::from(limit.max_per_window) {
        return Err(LnurlRepositoryError::RegistrationLimitExceeded);
    }
    Ok(())
}

/// The advisory-lock key standing for one pubkey's registration on one domain.
fn registration_lock_key(domain: &str, pubkey: &str) -> i64 {
    let mut engine = sha256::Hash::engine();
    engine.input(b"lnurl/registration/");
    engine.input(domain.as_bytes());
    // Neither field can hold a NUL, so no other pair hashes to this one.
    engine.input(b"\0");
    engine.input(pubkey.as_bytes());
    let digest = sha256::Hash::from_engine(engine);
    let mut key = [0u8; 8];
    key.copy_from_slice(&digest.as_byte_array()[..8]);
    i64::from_be_bytes(key)
}

/// Serialize this transaction against every other write to these pubkeys'
/// registrations, until it ends.
///
/// A pubkey holding no name yet has no row to lock, so without this two
/// registrations at once would both read that it holds nothing, and whichever
/// wrote second would drop the other's name without holding it.
///
/// Keys are taken in a fixed order, so two transactions locking the same pair
/// cannot each end up waiting on the key the other took.
async fn lock_registrations(
    tx: &mut sqlx::PgConnection,
    domain: &str,
    pubkeys: &[&str],
) -> Result<(), LnurlRepositoryError> {
    let mut keys: Vec<i64> = pubkeys
        .iter()
        .map(|pubkey| registration_lock_key(domain, pubkey))
        .collect();
    keys.sort_unstable();
    keys.dedup();

    for key in keys {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl crate::repository::LnurlRepository for LnurlRepository {
    async fn delete_user(
        &self,
        domain: &str,
        pubkey: &str,
        name: &str,
    ) -> Result<bool, LnurlRepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        lock_registrations(&mut tx, domain, &[pubkey]).await?;

        let result =
            sqlx::query("DELETE FROM users WHERE domain = $1 AND pubkey = $2 AND name = $3")
                .bind(domain)
                .bind(pubkey)
                .bind(name)
                .execute(&mut *tx)
                .await?;
        if result.rows_affected() == 0 {
            // Nothing was given up, so nothing is held. The tx rolls back on
            // drop.
            return Ok(false);
        }

        reserve_name(&mut *tx, domain, name, pubkey).await?;
        tx.commit()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        Ok(true)
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

    async fn upsert_user(
        &self,
        user: &User,
        limit: Option<RegistrationLimit>,
    ) -> Result<(), LnurlRepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;

        lock_registrations(&mut tx, &user.domain, &[&user.pubkey]).await?;

        // Read before the write overwrites it: taking a new name gives up the
        // one this pubkey holds now.
        let previous = name_of(&mut *tx, &user.domain, &user.pubkey).await?;

        // Written before the hold is checked. Taking the name in the unique
        // index is what serializes this against a release of the same name, so
        // a release that commits first is always visible below rather than
        // being read as absent and overtaken.
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
        .execute(&mut *tx)
        .await?;

        if reserved_for_other(&mut *tx, &user.domain, &user.name, &user.pubkey).await? {
            // The tx rolls back on drop, so the name stays as the hold left it.
            return Err(LnurlRepositoryError::NameReserved);
        }
        // Whatever hold stood on this name was this pubkey's own, or had
        // lapsed; either way holding it no longer means anything.
        sqlx::query("DELETE FROM released_names WHERE domain = $1 AND name = $2")
            .bind(&user.domain)
            .bind(&user.name)
            .execute(&mut *tx)
            .await?;

        let name_changed = previous.as_deref() != Some(user.name.as_str());
        if let Some(previous) = previous
            && previous != user.name
        {
            reserve_name(&mut *tx, &user.domain, &previous, &user.pubkey).await?;
        }

        if name_changed && let Some(limit) = limit {
            record_registration(&mut tx, &user.domain, &user.pubkey, limit).await?;
        }

        tx.commit()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        Ok(())
    }

    async fn name_status(
        &self,
        domain: &str,
        name: &str,
        asking_pubkey: Option<&str>,
    ) -> Result<NameStatus, LnurlRepositoryError> {
        let (taken, reserved): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM users WHERE domain = $1 AND name = $2)
                  , EXISTS(SELECT 1 FROM released_names
                           WHERE domain = $1 AND name = $2
                             AND ($4::text IS NULL OR pubkey <> $4)
                             AND (reclaimable_from IS NULL OR reclaimable_from > $3))",
        )
        .bind(domain)
        .bind(name)
        .bind(now())
        .bind(asking_pubkey)
        .fetch_one(&self.pool)
        .await?;

        Ok(match (taken, reserved) {
            (true, _) => NameStatus::Taken,
            (false, true) => NameStatus::Reserved,
            (false, false) => NameStatus::Free,
        })
    }

    async fn transfer_username(
        &self,
        transfer: TransferRequest<'_>,
        claim: crate::repository::StatementClaim<'_>,
    ) -> Result<(), LnurlRepositoryError> {
        let TransferRequest {
            domain,
            from_pubkey,
            to_pubkey,
            username,
            description,
        } = transfer;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        // Both sides, before the claim below: transactions that take the
        // registration locks first and the claim second never hold one while
        // waiting for the other in the opposite order.
        lock_registrations(&mut tx, domain, &[from_pubkey, to_pubkey]).await?;

        // Claim before performing, so a concurrent duplicate blocks on this key
        // until this transaction settles and then sees the outcome.
        if !claim_statement(&mut *tx, claim.hash, "transfer", claim.expires_at).await? {
            return Err(LnurlRepositoryError::StatementAlreadyUsed);
        }

        let source_name: Option<(String,)> =
            sqlx::query_as("DELETE FROM users WHERE domain = $1 AND pubkey = $2 RETURNING name")
                .bind(domain)
                .bind(from_pubkey)
                .fetch_optional(&mut *tx)
                .await?;
        match source_name {
            Some((name,)) if name == username => {}
            // Source pubkey doesn't currently own this username. The tx is
            // rolled back on drop, so the speculative DELETE and the claim are
            // both undone.
            _ => return Err(LnurlRepositoryError::SourceNotOwner),
        }

        // Accepting the transfer gives up whatever name the target holds now.
        let target_previous = name_of(&mut *tx, domain, to_pubkey).await?;

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

        if let Some(previous) = target_previous
            && previous != username
        {
            reserve_name(&mut *tx, domain, &previous, to_pubkey).await?;
        }

        tx.commit()
            .await
            .map_err(|e| LnurlRepositoryError::General(e.into()))?;
        Ok(())
    }

    async fn claim_signed_message(
        &self,
        statement_hash: &[u8],
        route: &str,
        expires_at: i64,
    ) -> Result<bool, LnurlRepositoryError> {
        claim_statement(&self.pool, statement_hash, route, expires_at).await
    }

    async fn delete_expired_signed_messages(&self, now: i64) -> Result<u64, LnurlRepositoryError> {
        let result = sqlx::query("DELETE FROM used_signed_messages WHERE expires_at < $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn delete_old_registrations(&self, cutoff: i64) -> Result<u64, LnurlRepositoryError> {
        let result = sqlx::query("DELETE FROM address_registrations WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
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

    async fn list_domains(&self) -> Result<Vec<DomainConfig>, LnurlRepositoryError> {
        let rows = sqlx::query(
            "SELECT d.domain, a.api_key, a.jwt \
             FROM allowed_domains d \
             LEFT JOIN domain_attribution a ON a.domain = d.domain",
        )
        .fetch_all(&self.pool)
        .await?;

        let domains = rows
            .into_iter()
            .map(|row| {
                Ok(DomainConfig {
                    domain: row.try_get(0)?,
                    api_key: row.try_get(1)?,
                    jwt: row.try_get(2)?,
                })
            })
            .collect::<Result<Vec<DomainConfig>, sqlx::Error>>()?;

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

    async fn set_domain_jwt(&self, domain: &str, jwt: &str) -> Result<(), LnurlRepositoryError> {
        sqlx::query("UPDATE domain_attribution SET jwt = $2 WHERE domain = $1")
            .bind(domain)
            .bind(jwt)
            .execute(&self.pool)
            .await?;
        Ok(())
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
                    i.domain,
                    z.zap_request
             FROM invoices i
             LEFT JOIN users u ON u.pubkey = i.user_pubkey AND u.domain = i.domain
             LEFT JOIN sender_comments sc ON sc.payment_hash = i.payment_hash
             LEFT JOIN zaps z ON z.payment_hash = i.payment_hash
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
                    nostr_zap_request: row.try_get(9)?,
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

#[cfg(test)]
mod postgres_tests {
    use crate::repository::shared_tests;
    use crate::test_support::test_pool;

    /// Seed `a.com` with an api key the way admins do: allowlist it, then set its
    /// key in the attribution table.
    async fn seed_domain_with_api_key(pool: &sqlx::PgPool) {
        sqlx::query("INSERT INTO allowed_domains (domain) VALUES ('a.com')")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO domain_attribution (domain, api_key) VALUES ('a.com', 'key-a')")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_domains_surfaces_api_keys() {
        let pool = test_pool("list_domains_surfaces_api_keys").await;
        seed_domain_with_api_key(&pool).await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::list_domains_surfaces_api_keys(&db).await;
    }

    #[tokio::test]
    async fn set_domain_jwt_round_trips() {
        let pool = test_pool("set_domain_jwt_round_trips").await;
        seed_domain_with_api_key(&pool).await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::set_domain_jwt_round_trips(&db).await;
    }

    #[tokio::test]
    async fn registering_taken_name_with_other_pubkey_is_rejected() {
        let pool = test_pool("registering_taken_name_rejected").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::registering_taken_name_with_other_pubkey_is_rejected(&db).await;
    }

    #[tokio::test]
    async fn deleting_a_name_the_pubkey_no_longer_holds_is_a_no_op() {
        let pool = test_pool("deleting_name_not_held_no_op").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::deleting_a_name_the_pubkey_no_longer_holds_is_a_no_op(&db).await;
    }

    #[tokio::test]
    async fn a_statement_is_claimable_once() {
        let pool = test_pool("claim_statement_twice").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_statement_is_claimable_once(&db).await;
    }

    #[tokio::test]
    async fn a_transfer_runs_once() {
        let pool = test_pool("transfer_runs_once").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_transfer_runs_once(&db).await;
    }

    #[tokio::test]
    async fn a_failed_transfer_stays_retryable() {
        let pool = test_pool("failed_transfer_retryable").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_failed_transfer_stays_retryable(&db).await;
    }

    #[tokio::test]
    async fn pruning_removes_only_expired_claims() {
        let pool = test_pool("prune_expired_claims").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::pruning_removes_only_expired_claims(&db).await;
    }

    #[tokio::test]
    async fn a_transfer_pair_is_spendable_once_across_domains() {
        let pool = test_pool("transfer_pair_spendable_once").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_transfer_pair_is_spendable_once_across_domains(&db).await;
    }

    #[tokio::test]
    async fn a_released_name_is_held_for_the_pubkey_that_released_it() {
        let pool = test_pool("released_name_is_held").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_released_name_is_held_for_the_pubkey_that_released_it(&db).await;
    }

    #[tokio::test]
    async fn registering_another_name_holds_the_one_left_behind() {
        let pool = test_pool("renaming_holds_old_name").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::registering_another_name_holds_the_one_left_behind(&db).await;
    }

    #[tokio::test]
    async fn registering_twice_at_once_holds_the_name_that_loses() {
        let pool = test_pool("registering_twice_at_once").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::registering_twice_at_once_holds_the_name_that_loses(&db).await;
    }

    #[tokio::test]
    async fn a_transfer_holds_the_name_the_target_gave_up() {
        let pool = test_pool("transfer_holds_target_name").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_transfer_holds_the_name_the_target_gave_up(&db).await;
    }

    #[tokio::test]
    async fn the_registration_limit_bounds_name_changes() {
        let pool = test_pool("limit_bounds_name_changes").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::the_registration_limit_bounds_name_changes(&db).await;
    }

    #[tokio::test]
    async fn re_registering_the_held_name_is_not_counted() {
        let pool = test_pool("limit_rereg_not_counted").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::re_registering_the_held_name_is_not_counted(&db).await;
    }

    #[tokio::test]
    async fn registrations_outside_the_window_do_not_count() {
        let pool = test_pool("limit_window_expiry").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::registrations_outside_the_window_do_not_count(&db).await;
    }

    #[tokio::test]
    async fn a_refused_registration_does_not_consume_quota() {
        let pool = test_pool("limit_refused_no_quota").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_refused_registration_does_not_consume_quota(&db).await;
    }

    #[tokio::test]
    async fn reclaiming_a_reserved_name_consumes_quota() {
        let pool = test_pool("limit_reclaim_counts").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::reclaiming_a_reserved_name_consumes_quota(&db).await;
    }

    #[tokio::test]
    async fn a_transfer_ignores_the_registration_limit() {
        let pool = test_pool("limit_transfer_ignores").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::a_transfer_ignores_the_registration_limit(&db).await;
    }

    #[tokio::test]
    async fn pruning_removes_only_old_registrations() {
        let pool = test_pool("limit_prune_old_rows").await;
        let db = super::LnurlRepository::new(pool);
        shared_tests::pruning_removes_only_old_registrations(&db).await;
    }

    /// A hold whose `reclaimable_from` has passed stops standing in anyone's
    /// way. Nothing writes a non-NULL `reclaimable_from` yet, so the row is
    /// seeded directly: the read side is what a cooldown policy would build on.
    #[tokio::test]
    async fn a_lapsed_hold_lets_anyone_register_the_name() {
        use crate::repository::{LnurlRepository, NameStatus};

        let pool = test_pool("lapsed_hold_frees_name").await;
        let db = super::LnurlRepository::new(pool.clone());
        let domain = "lapsed.com";

        sqlx::query(
            "INSERT INTO released_names (domain, name, pubkey, released_at, reclaimable_from)
             VALUES ($1, 'jane', 'jjjj', 0, 1)",
        )
        .bind(domain)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            db.name_status(domain, "jane", None).await.unwrap(),
            NameStatus::Free,
            "a hold whose time has passed must read as free"
        );
        db.upsert_user(
            &crate::user::User {
                domain: domain.into(),
                pubkey: "kkkk".into(),
                name: "jane".into(),
                description: "jane".into(),
            },
            None,
        )
        .await
        .expect("a lapsed hold must not refuse the registration");
        assert_eq!(
            db.get_user_by_name(domain, "jane")
                .await
                .unwrap()
                .map(|u| u.pubkey),
            Some("kkkk".to_string())
        );
    }
}
