use lnurl_models::ListMetadataMetadata;

use crate::user::User;
use crate::zap::Zap;

#[derive(Debug, thiserror::Error)]
pub enum LnurlRepositoryError {
    #[error("name taken")]
    NameTaken,
    /// The name is held for the pubkey that released it, and no one else may
    /// register it.
    #[error("name reserved")]
    NameReserved,
    #[error("source user does not own this username")]
    SourceNotOwner,
    /// The signed statement authorizing this request has already been acted on.
    #[error("statement already used")]
    StatementAlreadyUsed,
    /// The pubkey has registered as many addresses as its limit allows inside
    /// the counting window.
    #[error("registration limit exceeded")]
    RegistrationLimitExceeded,
    #[error("database error: {0}")]
    General(anyhow::Error),
}

/// A cap on the successful registrations one pubkey may perform in one domain
/// inside a rolling window.
#[derive(Debug, Clone, Copy)]
pub struct RegistrationLimit {
    pub max_per_window: u32,
    pub window_secs: u64,
}

impl RegistrationLimit {
    /// The production window: one rolling day.
    pub const DAY_SECS: u64 = 86_400;

    pub fn daily(max_per_window: u32) -> Self {
        Self {
            max_per_window,
            window_secs: Self::DAY_SECS,
        }
    }
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

/// Start the background pruner for expired signed-message claims and for
/// registration-log rows that have aged out of the counting window.
///
/// A claim only needs to outlive the window in which its message's timestamp is
/// still accepted. Registration rows are pruned on `registration_limit`'s own
/// window: pruning sooner would hand back quota the count still charges for.
/// With no limit, nothing writes new rows, and the default window clears what
/// an earlier run left behind.
pub fn start_claim_cleanup_processor<DB>(db: DB, registration_limit: Option<RegistrationLimit>)
where
    DB: LnurlRepository + Send + Sync + 'static,
{
    let registration_window =
        registration_limit.map_or(RegistrationLimit::DAY_SECS, |limit| limit.window_secs);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLAIM_CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            let now = crate::time::now();
            match db.delete_expired_signed_messages(now).await {
                Ok(0) => {}
                Ok(count) => tracing::debug!("pruned {count} expired signed-message claims"),
                Err(e) => tracing::error!("failed to prune signed-message claims: {e}"),
            }
            // A window too long to express prunes nothing, which is the
            // side to fail on: the count keeps charging for every row.
            let cutoff = now.saturating_sub(i64::try_from(registration_window).unwrap_or(i64::MAX));
            match db.delete_old_registrations(cutoff).await {
                Ok(0) => {}
                Ok(count) => tracing::debug!("pruned {count} old registration-log rows"),
                Err(e) => tracing::error!("failed to prune the registration log: {e}"),
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

/// What stands between a name and whoever wants to register it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameStatus {
    /// No one owns the name and no one holds it.
    Free,
    /// A pubkey owns the name now.
    Taken,
    /// The name is held for the pubkey that gave it up, who alone may register
    /// it again.
    Reserved,
}

/// Who a username moves between, and what the target's row says afterwards.
pub struct TransferRequest<'a> {
    pub domain: &'a str,
    pub from_pubkey: &'a str,
    pub to_pubkey: &'a str,
    pub username: &'a str,
    pub description: &'a str,
}

#[async_trait::async_trait]
pub trait LnurlRepository {
    /// Delete `pubkey`'s row in `domain`, but only while it still holds `name`,
    /// and hold `name` for it. Returns whether a row was removed.
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

    /// Give `user.pubkey` the name it asks for, releasing whichever name it
    /// holds now.
    ///
    /// Returns [`LnurlRepositoryError::NameReserved`] if the name is held for
    /// another pubkey, and clears the reservation when `user.pubkey` is the one
    /// it is held for.
    ///
    /// With a `limit`, a registration that changes the held name counts against
    /// it and is refused with
    /// [`LnurlRepositoryError::RegistrationLimitExceeded`] once the pubkey has
    /// used up its window on this domain. Re-registering the held name (e.g. a
    /// description update) never counts, and a refused registration leaves no
    /// trace.
    async fn upsert_user(
        &self,
        user: &User,
        limit: Option<RegistrationLimit>,
    ) -> Result<(), LnurlRepositoryError>;

    /// Whether `name` in `domain` is owned, held, or free, answered for
    /// `asking_pubkey`: a hold that pubkey may still reclaim reads as
    /// [`NameStatus::Free`], since it is free for the one asking.
    ///
    /// `None` asks on nobody's behalf and every hold stands, which is the only
    /// answer a caller that cannot prove which pubkey is asking may have: the
    /// reply would otherwise say who a name is held for.
    async fn name_status(
        &self,
        domain: &str,
        name: &str,
        asking_pubkey: Option<&str>,
    ) -> Result<NameStatus, LnurlRepositoryError>;

    /// Atomically transfer ownership of `transfer.username` from the source
    /// pubkey to the target, replacing any existing row for the target and
    /// releasing the name that row held.
    /// Returns [`LnurlRepositoryError::SourceNotOwner`] if the source does not
    /// currently own the username in that domain.
    ///
    /// Claims `claim` in the same transaction and returns
    /// [`LnurlRepositoryError::StatementAlreadyUsed`] if it was already claimed.
    /// A transfer that fails leaves no claim behind, so the statement stays
    /// actionable.
    async fn transfer_username(
        &self,
        transfer: TransferRequest<'_>,
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

    /// Delete registration-log rows created before `cutoff`, returning how many
    /// were removed.
    async fn delete_old_registrations(&self, cutoff: i64) -> Result<u64, LnurlRepositoryError>;

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
    use super::{
        LnurlRepository, LnurlRepositoryError, NameStatus, RegistrationLimit, StatementClaim,
        TransferRequest,
    };
    use crate::user::User;

    /// A user of `domain` holding `name`, described by their own name.
    fn user(domain: &str, pubkey: &str, name: &str) -> User {
        User {
            domain: domain.into(),
            pubkey: pubkey.into(),
            name: name.into(),
            description: name.into(),
        }
    }

    /// Register `name` to `pubkey`, holding whatever it held before.
    async fn register<DB>(
        db: &DB,
        domain: &str,
        pubkey: &str,
        name: &str,
    ) -> Result<(), LnurlRepositoryError>
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.upsert_user(&user(domain, pubkey, name), None).await
    }

    /// Upserting a name already owned by a different pubkey returns `NameTaken`
    /// and leaves the existing owner's row intact, rather than replacing it.
    pub async fn registering_taken_name_with_other_pubkey_is_rejected<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        register(db, "a.com", "aaaa", "alice").await.unwrap();

        let result = db
            .upsert_user(
                &User {
                    domain: "a.com".into(),
                    pubkey: "bbbb".into(),
                    name: "alice".into(),
                    description: "bob".into(),
                },
                None,
            )
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
        register(db, "a.com", "dddd", "dave").await.unwrap();

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
        assert_eq!(
            db.name_status("a.com", "erin", None).await.unwrap(),
            NameStatus::Free,
            "a delete that removed nothing must not hold a name"
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

    /// A transfer of `username` between the two pubkeys on `domain`.
    fn transfer<'a>(
        domain: &'a str,
        from: &'a str,
        to: &'a str,
        username: &'a str,
    ) -> TransferRequest<'a> {
        TransferRequest {
            domain,
            from_pubkey: from,
            to_pubkey: to,
            username,
            description: username,
        }
    }

    /// A transfer runs once: presenting the same source, target and username
    /// again is refused, since nothing bounds the pair that authorized it in
    /// time.
    pub async fn a_transfer_runs_once<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "transfer-once.com";
        register(db, domain, "aaaa", "amy").await.unwrap();

        let there = transfer_statement("aaaa", "bbbb", "amy");
        let back = transfer_statement("bbbb", "aaaa", "amy");

        db.transfer_username(
            transfer(domain, "aaaa", "bbbb", "amy"),
            transfer_claim(&there),
        )
        .await
        .unwrap();

        // Hand the name back, so only the claim stands between the pair and a
        // second run.
        db.transfer_username(
            transfer(domain, "bbbb", "aaaa", "amy"),
            transfer_claim(&back),
        )
        .await
        .unwrap();

        let replayed = db
            .transfer_username(
                transfer(domain, "aaaa", "bbbb", "amy"),
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
                transfer(domain, "cccc", "dddd", "cara"),
                transfer_claim(&statement),
            )
            .await;
        assert!(
            matches!(failed, Err(LnurlRepositoryError::SourceNotOwner)),
            "expected SourceNotOwner, got {failed:?}"
        );

        register(db, domain, "cccc", "cara").await.unwrap();

        db.transfer_username(
            transfer(domain, "cccc", "dddd", "cara"),
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
            register(db, domain, "hhhh", "holly").await.unwrap();
        }

        let statement = transfer_statement("hhhh", "iiii", "holly");
        db.transfer_username(
            transfer("spend-a.com", "hhhh", "iiii", "holly"),
            transfer_claim(&statement),
        )
        .await
        .expect("the first transfer must succeed");

        assert!(
            matches!(
                db.transfer_username(
                    transfer("spend-b.com", "hhhh", "iiii", "holly"),
                    transfer_claim(&statement),
                )
                .await,
                Err(super::LnurlRepositoryError::StatementAlreadyUsed)
            ),
            "the same pair must not transfer the name on a second domain"
        );
    }

    /// A name its owner gives up is held for them: no one else may register it,
    /// and taking it back frees it again.
    pub async fn a_released_name_is_held_for_the_pubkey_that_released_it<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "released.com";
        register(db, domain, "aaaa", "alice").await.unwrap();
        assert!(db.delete_user(domain, "aaaa", "alice").await.unwrap());

        assert_eq!(
            db.name_status(domain, "alice", None).await.unwrap(),
            NameStatus::Reserved,
            "a released name must not read as free"
        );
        assert_eq!(
            db.name_status(domain, "alice", Some("aaaa")).await.unwrap(),
            NameStatus::Free,
            "the name must read as free to the pubkey it is held for"
        );
        assert_eq!(
            db.name_status(domain, "alice", Some("bbbb")).await.unwrap(),
            NameStatus::Reserved,
            "the hold must stand for every other pubkey"
        );
        let sniped = register(db, domain, "bbbb", "alice").await;
        assert!(
            matches!(sniped, Err(LnurlRepositoryError::NameReserved)),
            "expected NameReserved, got {sniped:?}"
        );
        assert!(
            db.get_user_by_name(domain, "alice")
                .await
                .unwrap()
                .is_none(),
            "the refused registration must not have created a row"
        );

        register(db, domain, "aaaa", "alice")
            .await
            .expect("the pubkey that released the name must be able to take it back");
        assert_eq!(
            db.name_status(domain, "alice", None).await.unwrap(),
            NameStatus::Taken,
            "taking a name back must clear the hold on it"
        );
    }

    /// Registering a second name releases the first, which is the same hold as
    /// unregistering it: the name a pubkey walks away from never goes to a
    /// stranger, whichever route it walked away through.
    pub async fn registering_another_name_holds_the_one_left_behind<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "renamed.com";
        register(db, domain, "cccc", "carol").await.unwrap();
        register(db, domain, "cccc", "caroline").await.unwrap();

        assert_eq!(
            db.get_user_by_pubkey(domain, "cccc")
                .await
                .unwrap()
                .map(|u| u.name),
            Some("caroline".to_string())
        );
        let sniped = register(db, domain, "dddd", "carol").await;
        assert!(
            matches!(sniped, Err(LnurlRepositoryError::NameReserved)),
            "expected NameReserved, got {sniped:?}"
        );

        register(db, domain, "cccc", "carol")
            .await
            .expect("the pubkey that left the name behind must be able to go back to it");
        assert_eq!(
            db.name_status(domain, "caroline", None).await.unwrap(),
            NameStatus::Reserved,
            "going back must hold the name being left in turn"
        );
    }

    /// Two registrations by one pubkey at once leave it holding one name and
    /// the other held for it, whichever of them lands second. A pubkey with no
    /// row yet has none to lock, so nothing but the registration lock keeps the
    /// second registration from dropping the first name without holding it.
    pub async fn registering_twice_at_once_holds_the_name_that_loses<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "at-once.com";
        let first = {
            let db = db.clone();
            tokio::spawn(async move { register(&db, domain, "mmmm", "mary").await })
        };
        let second = {
            let db = db.clone();
            tokio::spawn(async move { register(&db, domain, "mmmm", "maud").await })
        };
        first.await.unwrap().expect("the first registration");
        second.await.unwrap().expect("the second registration");

        let held = db
            .get_user_by_pubkey(domain, "mmmm")
            .await
            .unwrap()
            .expect("one of the two names must be registered")
            .name;
        let lost = if held == "mary" { "maud" } else { "mary" };
        assert_eq!(
            db.name_status(domain, lost, None).await.unwrap(),
            NameStatus::Reserved,
            "the name that lost the race must be held, not left free"
        );
        assert_eq!(
            db.name_status(domain, lost, Some("mmmm")).await.unwrap(),
            NameStatus::Free,
            "the pubkey that lost it must be able to take it back"
        );
    }

    /// A transfer replaces the target's row, so the name the target held is
    /// released, and held for them the same way.
    pub async fn a_transfer_holds_the_name_the_target_gave_up<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "transfer-holds.com";
        register(db, domain, "eeee", "erin").await.unwrap();
        register(db, domain, "ffff", "fran").await.unwrap();

        let statement = transfer_statement("eeee", "ffff", "erin");
        db.transfer_username(
            transfer(domain, "eeee", "ffff", "erin"),
            transfer_claim(&statement),
        )
        .await
        .unwrap();

        assert_eq!(
            db.name_status(domain, "fran", None).await.unwrap(),
            NameStatus::Reserved,
            "the name the target gave up to accept the transfer must be held"
        );
        let sniped = register(db, domain, "gggg", "fran").await;
        assert!(
            matches!(sniped, Err(LnurlRepositoryError::NameReserved)),
            "expected NameReserved, got {sniped:?}"
        );
        assert_eq!(
            db.name_status(domain, "erin", None).await.unwrap(),
            NameStatus::Taken,
            "the transferred name is held by its new owner, not released"
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

    /// A limit inside a window that never expires during the test.
    fn limit(max_per_window: u32) -> RegistrationLimit {
        RegistrationLimit {
            max_per_window,
            window_secs: 3_600,
        }
    }

    /// Register `name` to `pubkey` under `limit`.
    async fn register_limited<DB>(
        db: &DB,
        domain: &str,
        pubkey: &str,
        name: &str,
        limit: Option<RegistrationLimit>,
    ) -> Result<(), LnurlRepositoryError>
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        db.upsert_user(&user(domain, pubkey, name), limit).await
    }

    /// The limit refuses the registration past `max_per_window` name changes,
    /// leaves the refused attempt without a trace, and binds one
    /// `(domain, pubkey)` only: other pubkeys and other domains keep their own
    /// budgets, and no limit enforces nothing.
    pub async fn the_registration_limit_bounds_name_changes<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit.com";
        register_limited(db, domain, "aaaa", "ada1", Some(limit(2)))
            .await
            .unwrap();
        register_limited(db, domain, "aaaa", "ada2", Some(limit(2)))
            .await
            .unwrap();

        let third = register_limited(db, domain, "aaaa", "ada3", Some(limit(2))).await;
        assert!(
            matches!(third, Err(LnurlRepositoryError::RegistrationLimitExceeded)),
            "expected RegistrationLimitExceeded, got {third:?}"
        );
        assert_eq!(
            db.get_user_by_pubkey(domain, "aaaa")
                .await
                .unwrap()
                .map(|u| u.name),
            Some("ada2".to_string()),
            "the refused registration must leave the held name alone"
        );
        assert_eq!(
            db.name_status(domain, "ada3", None).await.unwrap(),
            NameStatus::Free,
            "the refused registration must not have taken or held the name"
        );

        register_limited(db, domain, "bbbb", "bea", Some(limit(2)))
            .await
            .expect("another pubkey must have its own budget");
        register_limited(db, "limit-other.com", "aaaa", "ada1", Some(limit(2)))
            .await
            .expect("the same pubkey must have its own budget on another domain");
        register_limited(db, domain, "aaaa", "ada3", None)
            .await
            .expect("no limit must enforce nothing");
    }

    /// Re-registering the held name is not a name change: it never consumes
    /// quota, and it still applies the description.
    pub async fn re_registering_the_held_name_is_not_counted<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit-rereg.com";
        register_limited(db, domain, "cccc", "cleo", Some(limit(1)))
            .await
            .unwrap();

        db.upsert_user(
            &User {
                domain: domain.into(),
                pubkey: "cccc".into(),
                name: "cleo".into(),
                description: "new description".into(),
            },
            Some(limit(1)),
        )
        .await
        .expect("re-registering the held name must not consume quota");
        assert_eq!(
            db.get_user_by_pubkey(domain, "cccc")
                .await
                .unwrap()
                .map(|u| u.description),
            Some("new description".to_string())
        );

        let changed = register_limited(db, domain, "cccc", "cora", Some(limit(1))).await;
        assert!(
            matches!(
                changed,
                Err(LnurlRepositoryError::RegistrationLimitExceeded)
            ),
            "a name change must still count: got {changed:?}"
        );
    }

    /// Only registrations inside the window count: with a zero-length window
    /// every prior registration is out of it, so the same request passes.
    pub async fn registrations_outside_the_window_do_not_count<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit-window.com";
        register_limited(db, domain, "dddd", "dot1", Some(limit(1)))
            .await
            .unwrap();
        let refused = register_limited(db, domain, "dddd", "dot2", Some(limit(1))).await;
        assert!(matches!(
            refused,
            Err(LnurlRepositoryError::RegistrationLimitExceeded)
        ));

        let expired_window = Some(RegistrationLimit {
            max_per_window: 1,
            window_secs: 0,
        });
        register_limited(db, domain, "dddd", "dot2", expired_window)
            .await
            .expect("a registration outside the window must not count");
    }

    /// A registration refused for a taken name consumes no quota.
    pub async fn a_refused_registration_does_not_consume_quota<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit-refused.com";
        register(db, domain, "eeee", "elsa").await.unwrap();

        let taken = register_limited(db, domain, "ffff", "elsa", Some(limit(1))).await;
        assert!(
            matches!(taken, Err(LnurlRepositoryError::NameTaken)),
            "expected NameTaken, got {taken:?}"
        );
        register_limited(db, domain, "ffff", "faye", Some(limit(1)))
            .await
            .expect("the refused attempt must not have consumed the quota");
    }

    /// Reclaiming a name reserved for the pubkey is a name change like any
    /// other. Exempting it would let a pubkey flap between two names it has
    /// held without ever hitting the limit.
    pub async fn reclaiming_a_reserved_name_consumes_quota<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit-reclaim.com";
        register_limited(db, domain, "gggg", "gwen", Some(limit(2)))
            .await
            .unwrap();
        register_limited(db, domain, "gggg", "gail", Some(limit(2)))
            .await
            .unwrap();
        assert_eq!(
            db.name_status(domain, "gwen", None).await.unwrap(),
            NameStatus::Reserved
        );

        let reclaimed = register_limited(db, domain, "gggg", "gwen", Some(limit(2))).await;
        assert!(
            matches!(
                reclaimed,
                Err(LnurlRepositoryError::RegistrationLimitExceeded)
            ),
            "expected RegistrationLimitExceeded, got {reclaimed:?}"
        );
    }

    /// A transfer target at its registration limit still receives the name:
    /// a transfer moves an existing address rather than consuming a new one.
    pub async fn a_transfer_ignores_the_registration_limit<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit-transfer.com";
        register_limited(db, domain, "hhhh", "hana", Some(limit(1)))
            .await
            .unwrap();
        register(db, domain, "iiii", "iris").await.unwrap();

        let statement = transfer_statement("iiii", "hhhh", "iris");
        db.transfer_username(
            transfer(domain, "iiii", "hhhh", "iris"),
            transfer_claim(&statement),
        )
        .await
        .expect("a transfer to a pubkey at its limit must succeed");
        assert_eq!(
            db.get_user_by_pubkey(domain, "hhhh")
                .await
                .unwrap()
                .map(|u| u.name),
            Some("iris".to_string())
        );
    }

    /// Pruning drops only rows created before the cutoff, and a pruned budget
    /// frees the quota.
    pub async fn pruning_removes_only_old_registrations<DB>(db: &DB)
    where
        DB: LnurlRepository + Clone + Send + Sync + 'static,
    {
        let domain = "limit-prune.com";
        register_limited(db, domain, "jjjj", "june1", Some(limit(2)))
            .await
            .unwrap();
        register_limited(db, domain, "jjjj", "june2", Some(limit(2)))
            .await
            .unwrap();

        assert_eq!(
            db.delete_old_registrations(0).await.unwrap(),
            0,
            "a cutoff in the past must prune nothing"
        );
        let refused = register_limited(db, domain, "jjjj", "june3", Some(limit(2))).await;
        assert!(matches!(
            refused,
            Err(LnurlRepositoryError::RegistrationLimitExceeded)
        ));

        // Far in the future, so both rows are older than it. Other tests may
        // share the table, so assert at least this test's rows go.
        assert!(
            db.delete_old_registrations(i64::MAX).await.unwrap() >= 2,
            "a cutoff past every row must prune them"
        );
        register_limited(db, domain, "jjjj", "june3", Some(limit(2)))
            .await
            .expect("pruned registrations must not count");
    }
}
