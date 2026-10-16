//! Endorses the unallocated share of a wallet's Flashnet USDB rewards to the
//! wallet itself.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use flashnet::rewards::{MAX_RATIO_BPS, RewardsClient, USDB_MAINNET_TOKEN_IDENTIFIER};
use platform_utils::time::Duration;
use platform_utils::tokio;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::TokenBalance;
use crate::error::SdkError;
use crate::persist::{CachedRewardsDelegation, ObjectCacheRepository, Storage};
use crate::utils::time::now_secs;

/// Time limit for one delegation attempt.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a failed attempt holds off the next one.
const RETRY_INTERVAL_SECS: u64 = 60 * 60;

pub(crate) struct RewardsDelegator {
    client: RewardsClient,
    storage: Arc<dyn Storage>,
    holds_usdb: AtomicBool,
    /// The last known delegation state, read from storage or set by the last
    /// attempt. Unset until the first attempt finishes. Locked while an
    /// attempt runs.
    state: Mutex<Option<CachedRewardsDelegation>>,
}

impl RewardsDelegator {
    pub(crate) fn new(client: RewardsClient, storage: Arc<dyn Storage>) -> Self {
        Self {
            client,
            storage,
            holds_usdb: AtomicBool::new(false),
            state: Mutex::new(None),
        }
    }

    /// Records whether the wallet holds a positive USDB balance.
    pub(crate) fn set_token_balances(&self, token_balances: &HashMap<String, TokenBalance>) {
        let holds_usdb = token_balances
            .get(USDB_MAINNET_TOKEN_IDENTIFIER)
            .is_some_and(|b| b.balance > 0);
        self.holds_usdb.store(holds_usdb, Ordering::Relaxed);
    }

    /// Whether the wallet holds USDB, no attempt is running, and the known
    /// state allows one: not done, and any retry time has passed.
    pub(crate) fn should_delegate(&self) -> bool {
        self.holds_usdb.load(Ordering::Relaxed)
            && self.state.try_lock().is_ok_and(|state| match *state {
                Some(CachedRewardsDelegation::Done) => false,
                Some(CachedRewardsDelegation::RetryAfter(at)) => now_secs() >= at,
                None => true,
            })
    }

    /// Delegates the unallocated share within the time limit, one attempt at
    /// a time. An attempt that fails or times out holds off the next one for
    /// `RETRY_INTERVAL_SECS`.
    pub(crate) async fn delegate(&self) {
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        let lease =
            CachedRewardsDelegation::RetryAfter(now_secs().saturating_add(RETRY_INTERVAL_SECS));
        let outcome =
            match tokio::time::timeout(ATTEMPT_TIMEOUT, self.delegate_unallocated_rewards(lease))
                .await
            {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(e)) => {
                    warn!("Failed to delegate unallocated Flashnet rewards: {e:?}");
                    lease
                }
                Err(_) => {
                    warn!("Flashnet rewards delegation timed out");
                    lease
                }
            };
        *state = Some(outcome);
    }

    fn cache(&self) -> ObjectCacheRepository {
        ObjectCacheRepository::new(Arc::clone(&self.storage))
    }

    /// Endorses the share other endorsements leave unallocated to the wallet
    /// itself and records `Done`. Saves `lease` before any request, and returns
    /// the stored state when it is `Done` or a retry is pending.
    async fn delegate_unallocated_rewards(
        &self,
        lease: CachedRewardsDelegation,
    ) -> Result<CachedRewardsDelegation, SdkError> {
        let cache = self.cache();
        match cache.fetch_rewards_delegation().await? {
            Some(CachedRewardsDelegation::Done) => {
                debug!("Flashnet rewards delegation already recorded");
                return Ok(CachedRewardsDelegation::Done);
            }
            Some(CachedRewardsDelegation::RetryAfter(at)) if now_secs() < at => {
                debug!("Flashnet rewards delegation retries after {at}");
                return Ok(CachedRewardsDelegation::RetryAfter(at));
            }
            _ => {}
        }
        cache.save_rewards_delegation(&lease).await?;

        let own_pubkey = self.client.identity_public_key();
        let own_hex = own_pubkey.to_string();
        let endorsements = self.client.get_endorsements().await?.endorsements;
        let is_own = |to: &str| to.eq_ignore_ascii_case(&own_hex);
        let allocated_to_others = endorsements
            .iter()
            .filter(|e| !is_own(&e.to))
            .fold(0u16, |sum, e| sum.saturating_add(e.ratio_bps));
        let own_share = MAX_RATIO_BPS.saturating_sub(allocated_to_others);
        let current_own_share = endorsements
            .iter()
            .find(|e| is_own(&e.to))
            .map_or(0, |e| e.ratio_bps);

        if own_share > 0 && own_share != current_own_share {
            self.client
                .upsert_endorsement(&own_pubkey, own_share)
                .await?;
            info!(
                "Delegated {own_share} bps of Flashnet rewards to the wallet \
                 ({allocated_to_others} bps to others, {current_own_share} bps to the wallet before)"
            );
        } else {
            info!(
                "Flashnet rewards need no delegation: {allocated_to_others} bps to others, \
                 {current_own_share} bps to the wallet"
            );
        }

        cache
            .save_rewards_delegation(&CachedRewardsDelegation::Done)
            .await?;
        Ok(CachedRewardsDelegation::Done)
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use std::str::FromStr;

    use bitcoin::secp256k1::PublicKey;
    use flashnet::rewards::REWARDS_MAINNET_BASE_URL;
    use spark_wallet::SparkWallet;

    use super::*;
    use crate::TokenMetadata;

    /// Serves the Rewards API with a fixed endorsement list, and records each
    /// request as `METHOD url body`. Every request must go to `url`.
    struct MockRewards {
        url: String,
        endorsements: Vec<serde_json::Value>,
        /// The GET never answers.
        hang_lookup: bool,
        get_status: u16,
        post_status: u16,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl MockRewards {
        fn respond(
            &self,
            method: &str,
            url: &str,
            body: Option<String>,
        ) -> platform_utils::HttpResponse {
            self.requests
                .lock()
                .unwrap()
                .push(format!("{method} {url} {}", body.unwrap_or_default()));
            assert_eq!(url, self.url, "unexpected request");
            let status = if method == "POST" {
                self.post_status
            } else {
                self.get_status
            };
            let body = if status == 200 {
                serde_json::json!({ "endorser": "02", "endorsements": self.endorsements })
            } else {
                serde_json::json!({ "error": "rejected" })
            };
            platform_utils::HttpResponse {
                status,
                body: body.to_string(),
                headers: HashMap::new(),
            }
        }
    }

    #[macros::async_trait]
    impl platform_utils::HttpClient for MockRewards {
        async fn get(
            &self,
            url: String,
            _: Option<HashMap<String, String>>,
        ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
            let response = self.respond("GET", &url, None);
            if self.hang_lookup {
                std::future::pending::<()>().await;
            }
            Ok(response)
        }

        async fn post(
            &self,
            url: String,
            _: Option<HashMap<String, String>>,
            body: Option<String>,
        ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
            Ok(self.respond("POST", &url, body))
        }

        async fn delete(
            &self,
            url: String,
            _: Option<HashMap<String, String>>,
            body: Option<String>,
        ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
            Ok(self.respond("DELETE", &url, body))
        }
    }

    struct RewardsFixture {
        delegator: RewardsDelegator,
        storage: Arc<dyn Storage>,
        own_pubkey: String,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl RewardsFixture {
        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        /// The ratio of the endorsement posted to the wallet itself, after
        /// checking it is signed the way the Rewards API verifies: a 64-byte
        /// compact signature over the SHA-256 of the canonical YAML.
        fn posted_ratio_bps(&self) -> Option<u64> {
            use bitcoin::hashes::{Hash, sha256};
            use bitcoin::secp256k1::{Message, Secp256k1, ecdsa::Signature};

            self.requests().iter().find_map(|r| {
                let body = r.strip_prefix("POST ")?.split_once(' ')?.1;
                let body: serde_json::Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["endorser"], self.own_pubkey.as_str());
                assert_eq!(body["to"], self.own_pubkey.as_str());
                let ratio_bps = body["ratioBps"].as_u64().unwrap();

                let payload = format!(
                    "kind: rewards_endorsement\nversion: 1\naction: upsert\n\
                     endorser: \"{}\"\nto: \"{}\"\nratioBps: {ratio_bps}\ntimestampNonce: \"{}\"",
                    self.own_pubkey,
                    self.own_pubkey,
                    body["timestampNonce"].as_str().unwrap(),
                );
                let signature = hex::decode(body["signature"].as_str().unwrap()).unwrap();
                assert_eq!(signature.len(), 64, "the signature must be compact");
                let digest = sha256::Hash::hash(payload.as_bytes());
                Secp256k1::verification_only()
                    .verify_ecdsa(
                        &Message::from_digest(digest.to_byte_array()),
                        &Signature::from_compact(&signature).unwrap(),
                        &PublicKey::from_str(&self.own_pubkey).unwrap(),
                    )
                    .expect("the signature must cover the canonical payload");
                Some(ratio_bps)
            })
        }

        async fn recorded(&self) -> Option<CachedRewardsDelegation> {
            ObjectCacheRepository::new(Arc::clone(&self.storage))
                .fetch_rewards_delegation()
                .await
                .unwrap()
        }

        async fn delegated(&self) -> bool {
            self.recorded().await == Some(CachedRewardsDelegation::Done)
        }

        /// Whether a failure recorded a retry time that has not passed yet.
        async fn retry_pending(&self) -> bool {
            matches!(
                self.recorded().await,
                Some(CachedRewardsDelegation::RetryAfter(at)) if at > now_secs()
            )
        }

        async fn record(&self, value: CachedRewardsDelegation) {
            ObjectCacheRepository::new(Arc::clone(&self.storage))
                .save_rewards_delegation(&value)
                .await
                .unwrap();
        }

        /// Delegates if it should, as the runtimes do after a sync.
        async fn run(&self) -> bool {
            let should_delegate = self.delegator.should_delegate();
            if should_delegate {
                self.delegator.delegate().await;
            }
            should_delegate
        }
    }

    /// Stand-ins for the wallet's own pubkey in a fixture's endorsements, in
    /// lower and upper case hex.
    const SELF: &str = "self";
    const SELF_UPPER: &str = "SELF";

    const OTHER_PUBKEY: &str = "0257c86ea9ba12b11b24003aff7b546516dc31861b5b7b9ce5f96c9e3226d15e6d";
    const OTHER_PUBKEY_2: &str =
        "02d1caf75f1a175db2bf751f04b676f114f3c10d964c7601b5706a581e6a69b1be";

    /// What the Rewards API and the wallet look like to a fixture. The default
    /// is a wallet holding USDB, with no endorsements, whose requests all
    /// succeed.
    struct RewardsSetup {
        usdb_balance: Option<u128>,
        /// (recipient, ratio) pairs the GET returns.
        endorsements: &'static [(&'static str, u16)],
        hang_lookup: bool,
        get_status: u16,
        post_status: u16,
    }

    impl Default for RewardsSetup {
        fn default() -> Self {
            Self {
                usdb_balance: Some(1_000_000),
                endorsements: &[],
                hang_lookup: false,
                get_status: 200,
                post_status: 200,
            }
        }
    }

    fn token_balances(usdb_balance: Option<u128>) -> HashMap<String, TokenBalance> {
        usdb_balance
            .map(|balance| {
                (
                    USDB_MAINNET_TOKEN_IDENTIFIER.to_string(),
                    TokenBalance {
                        balance,
                        token_metadata: TokenMetadata {
                            identifier: USDB_MAINNET_TOKEN_IDENTIFIER.to_string(),
                            issuer_public_key: OTHER_PUBKEY.to_string(),
                            name: "USDB".to_string(),
                            ticker: "USDB".to_string(),
                            decimals: 6,
                            max_supply: 0,
                            is_freezable: false,
                        },
                    },
                )
            })
            .into_iter()
            .collect()
    }

    async fn rewards_delegator(setup: RewardsSetup) -> RewardsFixture {
        let mut dir = std::env::temp_dir();
        dir.push(format!("breez-test-rewards-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage: Arc<dyn Storage> = Arc::new(crate::SqliteStorage::new(&dir).unwrap());

        let signer = Arc::new(spark_wallet::SparkSignerAdapter::new(Arc::new(
            spark_wallet::DefaultSigner::new(&[7u8; 32], spark_wallet::Network::Regtest).unwrap(),
        )));
        let spark_wallet = Arc::new(
            SparkWallet::connect(
                spark_wallet::SparkWalletConfig::default_config(spark_wallet::Network::Regtest),
                signer,
            )
            .await
            .unwrap(),
        );
        let own_pubkey = spark_wallet.get_identity_public_key().to_string();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let http_client = Arc::new(MockRewards {
            url: format!("{REWARDS_MAINNET_BASE_URL}/v1/rewards/{own_pubkey}/endorsements"),
            endorsements: setup
                .endorsements
                .iter()
                .map(|&(to, ratio_bps)| {
                    let to = match to {
                        SELF => own_pubkey.clone(),
                        SELF_UPPER => own_pubkey.to_uppercase(),
                        to => to.to_string(),
                    };
                    serde_json::json!({ "to": to, "ratioBps": ratio_bps })
                })
                .collect(),
            hang_lookup: setup.hang_lookup,
            get_status: setup.get_status,
            post_status: setup.post_status,
            requests: Arc::clone(&requests),
        });
        let delegator = RewardsDelegator::new(
            RewardsClient::new(
                REWARDS_MAINNET_BASE_URL.to_string(),
                spark_wallet,
                http_client,
            ),
            Arc::clone(&storage),
        );
        delegator.set_token_balances(&token_balances(setup.usdb_balance));
        RewardsFixture {
            delegator,
            storage,
            own_pubkey,
            requests,
        }
    }

    #[tokio::test]
    async fn a_wallet_with_no_endorsements_delegates_everything_to_itself_once() {
        let fixture = rewards_delegator(RewardsSetup::default()).await;

        assert!(fixture.run().await);
        assert_eq!(fixture.posted_ratio_bps(), Some(10_000));
        assert!(fixture.delegated().await);

        assert!(!fixture.run().await, "done after the first attempt");
    }

    #[tokio::test]
    async fn a_wallet_without_usdb_keeps_its_attempt_until_it_holds_some() {
        let fixture = rewards_delegator(RewardsSetup {
            usdb_balance: None,
            ..Default::default()
        })
        .await;

        assert!(!fixture.run().await);
        assert!(fixture.requests().is_empty());

        fixture
            .delegator
            .set_token_balances(&token_balances(Some(1)));
        assert!(fixture.run().await);
        assert_eq!(fixture.posted_ratio_bps(), Some(10_000));
    }

    #[tokio::test]
    async fn a_zero_usdb_balance_counts_as_none() {
        let fixture = rewards_delegator(RewardsSetup {
            usdb_balance: Some(0),
            ..Default::default()
        })
        .await;

        assert!(!fixture.run().await);
        assert!(fixture.requests().is_empty());
    }

    #[tokio::test]
    async fn only_the_unallocated_share_is_delegated_to_the_wallet() {
        let fixture = rewards_delegator(RewardsSetup {
            endorsements: &[(OTHER_PUBKEY, 3000)],
            ..Default::default()
        })
        .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), Some(7000));
        assert!(fixture.delegated().await);
    }

    #[tokio::test]
    async fn a_stale_own_share_is_raised_to_the_unallocated_share() {
        let fixture = rewards_delegator(RewardsSetup {
            endorsements: &[(OTHER_PUBKEY, 3000), (SELF, 5000)],
            ..Default::default()
        })
        .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), Some(7000));
        assert!(fixture.delegated().await);
    }

    #[tokio::test]
    async fn a_correct_own_share_is_left_alone_whatever_its_case() {
        let fixture = rewards_delegator(RewardsSetup {
            endorsements: &[(OTHER_PUBKEY, 3000), (SELF_UPPER, 7000)],
            ..Default::default()
        })
        .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), None);
        assert!(fixture.delegated().await);
    }

    #[tokio::test]
    async fn a_fully_allocated_wallet_is_left_alone() {
        let fixture = rewards_delegator(RewardsSetup {
            endorsements: &[(OTHER_PUBKEY, 10_000)],
            ..Default::default()
        })
        .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), None);
        assert!(fixture.delegated().await);
    }

    #[tokio::test]
    async fn an_over_allocated_wallet_is_left_alone() {
        let fixture = rewards_delegator(RewardsSetup {
            endorsements: &[(OTHER_PUBKEY, 6000), (OTHER_PUBKEY_2, 6000)],
            ..Default::default()
        })
        .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), None);
        assert!(fixture.delegated().await);
    }

    #[tokio::test]
    async fn a_failed_lookup_records_a_retry_time() {
        let fixture = rewards_delegator(RewardsSetup {
            get_status: 500,
            ..Default::default()
        })
        .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), None);
        assert!(fixture.retry_pending().await);
        assert!(!fixture.run().await, "the retry time holds");
    }

    #[tokio::test]
    async fn a_pending_retry_time_holds_off_the_attempt() {
        let fixture = rewards_delegator(RewardsSetup::default()).await;
        fixture
            .record(CachedRewardsDelegation::RetryAfter(
                now_secs().saturating_add(60),
            ))
            .await;

        fixture.run().await;
        assert!(fixture.requests().is_empty());
    }

    #[tokio::test]
    async fn a_passed_retry_time_allows_the_attempt() {
        let fixture = rewards_delegator(RewardsSetup::default()).await;
        fixture
            .record(CachedRewardsDelegation::RetryAfter(
                now_secs().saturating_sub(1),
            ))
            .await;

        fixture.run().await;
        assert_eq!(fixture.posted_ratio_bps(), Some(10_000));
        assert!(fixture.delegated().await);
    }

    #[tokio::test]
    async fn a_rejected_delegation_waits_for_its_retry_time() {
        let fixture = rewards_delegator(RewardsSetup {
            post_status: 400,
            ..Default::default()
        })
        .await;

        assert!(fixture.run().await);
        assert!(fixture.retry_pending().await);

        let made = fixture.requests().len();
        assert!(!fixture.run().await);
        assert_eq!(fixture.requests().len(), made);
    }

    #[tokio::test]
    async fn an_instance_retries_once_its_retry_time_passes() {
        let fixture = rewards_delegator(RewardsSetup::default()).await;
        let passed = CachedRewardsDelegation::RetryAfter(now_secs().saturating_sub(1));
        fixture.record(passed).await;
        *fixture.delegator.state.try_lock().unwrap() = Some(passed);

        assert!(fixture.run().await);
        assert_eq!(fixture.posted_ratio_bps(), Some(10_000));
        assert!(fixture.delegated().await);
        assert!(!fixture.run().await, "done after the retry");
    }

    #[tokio::test]
    async fn a_cancelled_attempt_leaves_a_retry_time_and_ends_the_attempt() {
        let fixture = rewards_delegator(RewardsSetup {
            hang_lookup: true,
            ..Default::default()
        })
        .await;

        let attempt = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            fixture.delegator.delegate(),
        )
        .await;
        assert!(attempt.is_err(), "the attempt hangs until cancelled");
        assert!(fixture.retry_pending().await);

        let made = fixture.requests().len();
        assert!(fixture.run().await, "the cancelled attempt has ended");
        assert_eq!(fixture.requests().len(), made, "the retry time holds");
    }

    #[tokio::test]
    async fn an_attempt_in_progress_blocks_another() {
        let fixture = rewards_delegator(RewardsSetup {
            hang_lookup: true,
            ..Default::default()
        })
        .await;

        let first = fixture.delegator.delegate();
        tokio::pin!(first);
        tokio::select! {
            () = &mut first => panic!("the attempt hangs"),
            () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }

        assert!(!fixture.delegator.should_delegate());
        let made = fixture.requests().len();
        fixture.delegator.delegate().await;
        assert_eq!(fixture.requests().len(), made);
    }

    #[tokio::test]
    async fn a_known_pending_retry_time_skips_the_attempt() {
        let fixture = rewards_delegator(RewardsSetup::default()).await;
        *fixture.delegator.state.try_lock().unwrap() = Some(CachedRewardsDelegation::RetryAfter(
            now_secs().saturating_add(60),
        ));

        assert!(!fixture.run().await);
        assert!(fixture.requests().is_empty());
    }

    #[tokio::test]
    async fn a_recorded_delegation_is_not_repeated() {
        let fixture = rewards_delegator(RewardsSetup::default()).await;
        fixture.record(CachedRewardsDelegation::Done).await;

        fixture.run().await;
        assert!(fixture.requests().is_empty());
    }
}
