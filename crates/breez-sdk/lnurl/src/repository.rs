use lnurl_models::ListMetadataMetadata;

use crate::user::User;
use crate::zap::Zap;

#[derive(Debug, thiserror::Error)]
pub enum LnurlRepositoryError {
    #[error("name taken")]
    NameTaken,
    #[error("source user does not own this username")]
    SourceNotOwner,
    /// The signed statement authorizing this request has already been acted on.
    #[error("statement already used")]
    StatementAlreadyUsed,
    #[error("database error: {0}")]
    General(anyhow::Error),
}

pub struct LnurlSenderComment {
    pub comment: String,
    pub payment_hash: String,
    pub user_pubkey: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct Invoice {
    pub payment_hash: String,
    pub user_pubkey: String,
    pub invoice: String,
    pub preimage: Option<String>,
    pub invoice_expiry: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// The domain this invoice was created for, if any.
    pub domain: Option<String>,
    /// Amount received in satoshis (from the HTLC). NULL when unknown.
    pub amount_received_sat: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PendingZapReceipt {
    pub payment_hash: String,
    pub created_at: i64,
    pub retry_count: i32,
    pub next_retry_at: i64,
}

#[derive(Debug, Clone)]
pub struct DomainConfig {
    pub domain: String,
    /// The domain's own Breez API key, if set.
    pub api_key: Option<String>,
    /// The cached partner JWT if one has been fetched and persisted.
    pub jwt: Option<String>,
}

/// How often expired signed-message claims are pruned.
const CLAIM_CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_hours(1);

/// Start the background pruner for expired signed-message claims.
///
/// A claim only needs to outlive the window in which its message's timestamp is
/// still accepted.
pub fn start_claim_cleanup_processor<DB>(db: DB)
where
    DB: LnurlRepository + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLAIM_CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            match db.delete_expired_signed_messages(crate::time::now()).await {
                Ok(0) => {}
                Ok(count) => tracing::debug!("pruned {count} expired signed-message claims"),
                Err(e) => tracing::error!("failed to prune signed-message claims: {e}"),
            }
        }
    });
}

/// The statement an action records as acted on, so the signature that
/// authorized it cannot authorize a second one.
pub struct StatementClaim<'a> {
    pub hash: &'a [u8],
    /// When the row may be pruned.
    pub expires_at: i64,
}

#[async_trait::async_trait]
pub trait LnurlRepository {
    /// Delete `pubkey`'s row in `domain`, but only while it still holds `name`.
    /// Returns whether a row was removed.
    ///
    /// `name` is part of the condition so the caller's authorization check and
    /// the delete cannot disagree: a row that changed name in between is left
    /// alone rather than deleted on the strength of a stale read. The caller
    /// reads before deleting, so `false` means the row changed in between.
    async fn delete_user(
        &self,
        domain: &str,
        pubkey: &str,
        name: &str,
    ) -> Result<bool, LnurlRepositoryError>;
    async fn get_user_by_name(
        &self,
        domain: &str,
        name: &str,
    ) -> Result<Option<User>, LnurlRepositoryError>;
    async fn get_user_by_pubkey(
        &self,
        domain: &str,
        pubkey: &str,
    ) -> Result<Option<User>, LnurlRepositoryError>;
    async fn upsert_user(&self, user: &User) -> Result<(), LnurlRepositoryError>;

    /// Atomically transfer ownership of `username` in `domain` from `from_pubkey`
    /// to `to_pubkey`, replacing any existing row for `to_pubkey`.
    /// Returns [`LnurlRepositoryError::SourceNotOwner`] if `from_pubkey` does not
    /// currently own `username` in `domain`.
    ///
    /// Claims `claim` in the same transaction and returns
    /// [`LnurlRepositoryError::StatementAlreadyUsed`] if it was already claimed.
    /// A transfer that fails leaves no claim behind, so the statement stays
    /// actionable.
    async fn transfer_username(
        &self,
        domain: &str,
        from_pubkey: &str,
        to_pubkey: &str,
        username: &str,
        description: &str,
        claim: StatementClaim<'_>,
    ) -> Result<(), LnurlRepositoryError>;

    /// Claim `statement_hash` for `route`, returning whether this call claimed
    /// it. `false` means it was already claimed and must not be acted on again.
    /// `expires_at` is when the row may be pruned.
    ///
    /// The first caller to claim a statement wins, and the stored route is never
    /// overwritten.
    async fn claim_signed_message(
        &self,
        statement_hash: &[u8],
        route: &str,
        expires_at: i64,
    ) -> Result<bool, LnurlRepositoryError>;

    /// Delete claimed statements whose `expires_at` has passed.
    async fn delete_expired_signed_messages(&self, now: i64) -> Result<u64, LnurlRepositoryError>;

    async fn upsert_zap(&self, zap: &Zap) -> Result<(), LnurlRepositoryError>;
    async fn insert_lnurl_sender_comment(
        &self,
        comment: &LnurlSenderComment,
    ) -> Result<(), LnurlRepositoryError>;
    async fn get_metadata_by_pubkey(
        &self,
        pubkey: &str,
        offset: u32,
        limit: u32,
        updated_after: Option<i64>,
    ) -> Result<Vec<ListMetadataMetadata>, LnurlRepositoryError>;

    /// Get all allowed domains and their optional Breez API keys.
    async fn list_domains(&self) -> Result<Vec<DomainConfig>, LnurlRepositoryError>;

    /// Insert a domain if it doesn't already exist
    async fn add_domain(&self, domain: &str) -> Result<(), LnurlRepositoryError>;

    /// Store the cached partner JWT for a domain.
    async fn set_domain_jwt(&self, domain: &str, jwt: &str) -> Result<(), LnurlRepositoryError>;

    /// Insert or update an invoice
    async fn upsert_invoice(&self, invoice: &Invoice) -> Result<(), LnurlRepositoryError>;

    /// Get an invoice by payment hash
    async fn get_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<Option<Invoice>, LnurlRepositoryError>;

    /// Get both the zap and invoice for a payment hash in a single query
    async fn get_zap_and_invoice_by_payment_hash(
        &self,
        payment_hash: &str,
    ) -> Result<(Option<Zap>, Option<Invoice>), LnurlRepositoryError>;
    /// Insert a pending zap receipt into the queue
    async fn insert_pending_zap_receipt(
        &self,
        pending: &PendingZapReceipt,
    ) -> Result<(), LnurlRepositoryError>;

    /// Get pending zap receipts ready for processing (`next_retry_at` <= now),
    /// atomically claiming them. Items already claimed by another instance
    /// within the last 5 minutes are skipped.
    async fn take_pending_zap_receipts(
        &self,
        limit: u32,
    ) -> Result<Vec<PendingZapReceipt>, LnurlRepositoryError>;

    /// Update retry count and next retry time for a pending zap receipt
    async fn update_pending_zap_receipt_retry(
        &self,
        payment_hash: &str,
        retry_count: i32,
        next_retry_at: i64,
    ) -> Result<(), LnurlRepositoryError>;

    /// Delete a pending zap receipt from the queue
    async fn delete_pending_zap_receipt(
        &self,
        payment_hash: &str,
    ) -> Result<(), LnurlRepositoryError>;

    /// Get or create a setting. If the key doesn't exist, insert the default value.
    /// Returns the current value (either existing or newly inserted).
    async fn get_or_create_setting(
        &self,
        key: &str,
        default_value: &str,
    ) -> Result<String, LnurlRepositoryError>;

    /// Get data needed to build webhook payloads for the given payment hashes.
    /// Joins invoices, users, `sender_comments`, and `domain_webhooks`.
    /// Returns rows for invoices that have a domain and a preimage.
    async fn get_webhook_payloads(
        &self,
        payment_hashes: &[String],
    ) -> Result<Vec<WebhookPayloadData>, LnurlRepositoryError>;
}

/// Data returned by the webhook enqueue query.
pub struct WebhookPayloadData {
    pub payment_hash: String,
    pub user_pubkey: String,
    pub invoice: String,
    pub preimage: String,
    pub amount_received_sat: Option<i64>,
    pub lightning_address: Option<String>,
    pub sender_comment: Option<String>,
    pub domain: String,
}

/// Tests for the domain-attribution repository methods, generic over the
/// `LnurlRepository` implementation. Assertions look up domains by name rather
/// than by count, so they tolerate a shared test database with rows from other
/// tests.
#[cfg(test)]
pub mod shared_tests {
    use super::{LnurlRepository, LnurlRepositoryError, StatementClaim};
    use crate::user::User;

    /// Upserting a name already owned by a different pubkey returns `NameTaken`
    /// and leaves the existing owner's row intact, rather than replacing it.
    pub async fn registering_taken_name_with_other_pubkey_is_rejected<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.upsert_user(&User {
            domain: "a.com".into(),
            pubkey: "aaaa".into(),
            name: "alice".into(),
            description: "alice".into(),
        })
        .await
        .unwrap();

        let result = db
            .upsert_user(&User {
                domain: "a.com".into(),
                pubkey: "bbbb".into(),
                name: "alice".into(),
                description: "bob".into(),
            })
            .await;
        assert!(
            matches!(result, Err(LnurlRepositoryError::NameTaken)),
            "expected NameTaken, got {result:?}"
        );

        let owner = db
            .get_user_by_name("a.com", "alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            owner.pubkey, "aaaa",
            "existing owner was replaced, now resolves to pubkey {}",
            owner.pubkey
        );
    }

    /// `delete_user` removes the row only while it still holds the named
    /// address, so an unregister authorized for one name cannot delete another.
    ///
    /// Uses its own pubkey and name: the postgres harness shares one database
    /// across tests that run in parallel, and `users` is keyed by
    /// `(domain, pubkey)` with `name` unique per domain.
    pub async fn deleting_a_name_the_pubkey_no_longer_holds_is_a_no_op<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.upsert_user(&User {
            domain: "a.com".into(),
            pubkey: "dddd".into(),
            name: "dave".into(),
            description: "dave".into(),
        })
        .await
        .unwrap();

        let removed = db.delete_user("a.com", "dddd", "erin").await.unwrap();
        assert!(
            !removed,
            "deleting 'erin' must report that it removed nothing"
        );
        let held = db.get_user_by_pubkey("a.com", "dddd").await.unwrap();
        assert_eq!(
            held.map(|u| u.name),
            Some("dave".to_string()),
            "deleting 'erin' must not touch the 'dave' the pubkey holds"
        );

        let removed = db.delete_user("a.com", "dddd", "dave").await.unwrap();
        assert!(removed, "deleting the held name must report the removal");
        assert!(
            db.get_user_by_pubkey("a.com", "dddd")
                .await
                .unwrap()
                .is_none(),
            "deleting the held name must remove the row"
        );
    }

    /// A claimed statement is refused whichever route presents it next, so one
    /// signature is never acted on twice.
    pub async fn a_statement_is_claimable_once<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let statement = b"claim-twice-statement";

        assert!(
            db.claim_signed_message(statement, "register", 9_000)
                .await
                .unwrap()
        );
        assert!(
            !db.claim_signed_message(statement, "unregister", 9_000)
                .await
                .unwrap(),
            "another route must not be able to claim it"
        );
        assert!(
            !db.claim_signed_message(statement, "register", 9_000)
                .await
                .unwrap(),
            "the route holding the claim must not be able to claim it again"
        );
    }

    /// Pruning drops claims whose expiry has passed and keeps the rest.
    pub async fn pruning_removes_only_expired_claims<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let expired = b"prune-expired";
        let live = b"prune-live";
        let unbounded = b"prune-unbounded";

        db.claim_signed_message(expired, "register", 1_000)
            .await
            .unwrap();
        db.claim_signed_message(live, "register", 9_000)
            .await
            .unwrap();
        db.claim_signed_message(unbounded, "transfer", i64::MAX)
            .await
            .unwrap();

        db.delete_expired_signed_messages(5_000).await.unwrap();

        assert!(
            db.claim_signed_message(expired, "unregister", 9_000)
                .await
                .unwrap(),
            "an expired claim must be gone"
        );
        assert!(
            !db.claim_signed_message(live, "unregister", 9_000)
                .await
                .unwrap(),
            "a claim inside its window must survive"
        );

        // Far past any expiry a timestamped message can carry, so only a claim
        // meant to outlive every window survives this.
        db.delete_expired_signed_messages(i64::MAX - 1)
            .await
            .unwrap();
        assert!(
            !db.claim_signed_message(unbounded, "transfer", i64::MAX)
                .await
                .unwrap(),
            "a claim nothing bounds in time must never be pruned"
        );
    }

    /// The statement a transfer pair authorizes, standing in for what the route
    /// hashes: the source, the target and the username, and no domain.
    fn transfer_statement(from: &str, to: &str, username: &str) -> Vec<u8> {
        format!("transfer:{from}-{to}-{username}").into_bytes()
    }

    /// A transfer claim over `statement`, which nothing bounds in time.
    fn transfer_claim(statement: &[u8]) -> StatementClaim<'_> {
        StatementClaim {
            hash: statement,
            expires_at: i64::MAX,
        }
    }

    /// A transfer runs once: presenting the same source, target and username
    /// again is refused, since nothing bounds the pair that authorized it in
    /// time.
    pub async fn a_transfer_runs_once<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let seed = |pubkey: &str, name: &str| User {
            domain: "transfer-once.com".into(),
            pubkey: pubkey.into(),
            name: name.into(),
            description: String::new(),
        };
        db.upsert_user(&seed("aaaa", "amy")).await.unwrap();

        let there = transfer_statement("aaaa", "bbbb", "amy");
        let back = transfer_statement("bbbb", "aaaa", "amy");

        db.transfer_username(
            "transfer-once.com",
            "aaaa",
            "bbbb",
            "amy",
            "amy",
            transfer_claim(&there),
        )
        .await
        .unwrap();

        // Hand the name back, so only the claim stands between the pair and a
        // second run.
        db.transfer_username(
            "transfer-once.com",
            "bbbb",
            "aaaa",
            "amy",
            "amy",
            transfer_claim(&back),
        )
        .await
        .unwrap();

        let replayed = db
            .transfer_username(
                "transfer-once.com",
                "aaaa",
                "bbbb",
                "amy",
                "amy",
                transfer_claim(&there),
            )
            .await;
        assert!(
            matches!(replayed, Err(LnurlRepositoryError::StatementAlreadyUsed)),
            "expected StatementAlreadyUsed, got {replayed:?}"
        );
        assert_eq!(
            db.get_user_by_name("transfer-once.com", "amy")
                .await
                .unwrap()
                .unwrap()
                .pubkey,
            "aaaa",
            "the refused transfer must not have moved the name"
        );
    }

    /// A transfer that fails leaves no claim, so the same pair can be presented
    /// again once the source holds the name.
    pub async fn a_failed_transfer_stays_retryable<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "transfer-retry.com";
        let statement = transfer_statement("cccc", "dddd", "cara");
        let failed = db
            .transfer_username(
                domain,
                "cccc",
                "dddd",
                "cara",
                "cara",
                transfer_claim(&statement),
            )
            .await;
        assert!(
            matches!(failed, Err(LnurlRepositoryError::SourceNotOwner)),
            "expected SourceNotOwner, got {failed:?}"
        );

        db.upsert_user(&User {
            domain: domain.into(),
            pubkey: "cccc".into(),
            name: "cara".into(),
            description: String::new(),
        })
        .await
        .unwrap();

        db.transfer_username(
            domain,
            "cccc",
            "dddd",
            "cara",
            "cara",
            transfer_claim(&statement),
        )
        .await
        .expect("a transfer that never happened must stay retryable");
    }

    /// One pair authorizes one transfer wherever it is submitted. The signed
    /// message names no domain, so a source holding the name on several of them
    /// does not get a transfer on each.
    pub async fn a_transfer_pair_is_spendable_once_across_domains<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        for domain in ["spend-a.com", "spend-b.com"] {
            db.upsert_user(&User {
                domain: domain.into(),
                pubkey: "hhhh".into(),
                name: "holly".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        }

        let statement = transfer_statement("hhhh", "iiii", "holly");
        db.transfer_username(
            "spend-a.com",
            "hhhh",
            "iiii",
            "holly",
            "holly",
            transfer_claim(&statement),
        )
        .await
        .expect("the first transfer must succeed");

        assert!(
            matches!(
                db.transfer_username(
                    "spend-b.com",
                    "hhhh",
                    "iiii",
                    "holly",
                    "holly",
                    transfer_claim(&statement),
                )
                .await,
                Err(super::LnurlRepositoryError::StatementAlreadyUsed)
            ),
            "the same pair must not transfer the name on a second domain"
        );
    }

    /// `list_domains` surfaces a domain's `api_key` and reports `None` for one
    /// with no key, added via `add_domain`. The caller seeds `a.com` with an
    /// `api_key` (`key-a`) first, since setting a key is a direct row write with
    /// no trait method (admins manage keys out-of-band).
    pub async fn list_domains_surfaces_api_keys<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.add_domain("b.com").await.unwrap();

        let domains = db.list_domains().await.unwrap();
        let with_key = domains
            .iter()
            .find(|d| d.domain == "a.com")
            .expect("seeded domain with an api key");
        assert_eq!(with_key.api_key.as_deref(), Some("key-a"));
        let without_key = domains
            .iter()
            .find(|d| d.domain == "b.com")
            .expect("domain with no api key");
        assert_eq!(without_key.api_key, None);
    }

    /// `set_domain_jwt` updates the cached JWT of a domain with an api key
    /// (readable via `list_domains`); a domain with no attribution row is a
    /// no-op, not an error. The caller seeds `a.com` with an api key (allowlisted
    /// + `api_key` set) first, since a row can only be created by setting an api key.
    pub async fn set_domain_jwt_round_trips<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let before = db.list_domains().await.unwrap();
        assert_eq!(
            before.iter().find(|d| d.domain == "a.com").unwrap().jwt,
            None
        );

        db.set_domain_jwt("a.com", "tok").await.unwrap();
        // A domain with no attribution row updates zero rows, not an error.
        db.set_domain_jwt("missing.com", "x").await.unwrap();

        let after = db.list_domains().await.unwrap();
        assert_eq!(
            after
                .iter()
                .find(|d| d.domain == "a.com")
                .unwrap()
                .jwt
                .as_deref(),
            Some("tok")
        );
    }
}
