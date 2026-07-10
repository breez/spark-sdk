use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use breez_sdk_common::utils::now;
use platform_utils::HttpClient;
use platform_utils::time::Duration;
use platform_utils::tokio;
use platform_utils::tokio::sync::RwLock;
use serde::Deserialize;
use spark_wallet::{HeaderProvider, HeaderProviderError};
use tokio::sync::oneshot;
use tracing::{Instrument, warn};

use crate::persist::Storage;

const PARTNER_ID_HEADER: &str = "x-partner-jwt";
const KEY_BREEZ_JWT: &str = "breez_jwt";
const JWT_EXPIRY_GRACE_PERIOD_SECS: u64 = 60 * 5;
/// Stop serving a token this many seconds before its real `exp`, as a
/// clock-skew guard so we never hand out a token the server may already treat
/// as expired. Distinct from the 5-minute refresh lead in
/// [`JWT_EXPIRY_GRACE_PERIOD_SECS`]: the token stays servable through the
/// refresh window, and is only withheld in this final margin before expiry.
const JWT_SERVE_SKEW_SECS: u64 = 30;
/// Fallback refresh interval used only if a freshly-fetched token has no
/// parseable `exp` claim. In practice the server always returns a token with
/// an `exp`, so this path is for malformed-response robustness.
const JWT_FALLBACK_INTERVAL_SECS: u64 = 60;
const JWT_BACKOFF_MAX_SECS: u64 = 60 * 5;
const JWT_BREEZSERVER_URL: &str = "https://nd1.breez.technology:443";

#[derive(Deserialize)]
struct Jwt {
    exp: u64,
}

#[derive(Deserialize)]
struct JwtServerResponse {
    token: String,
}

/// In-memory cache entry: the JWT and its parsed `exp` claim, stored
/// together so neither `headers()` nor the refresh loop has to re-parse the
/// JWT on every call.
struct CachedToken {
    token: String,
    /// Unix timestamp (seconds) at which the JWT expires. `0` is used as a
    /// sentinel for tokens whose `exp` claim couldn't be parsed — those are
    /// treated as already expired by [`is_expired`] and trigger a quick
    /// refresh via [`next_refresh_after_success`].
    exp: u64,
}

struct Inner {
    token: RwLock<Option<CachedToken>>,
    api_key: String,
    /// Bound once by [`BreezJwtHeaderProvider::start`] before the refresh task
    /// spawns; empty when the provider persists nothing (in-memory only).
    storage: OnceLock<Arc<dyn Storage>>,
    http_client: Arc<dyn HttpClient>,
}

/// Header provider that injects the Breez partner JWT (`x-partner-jwt`) into
/// outgoing SO requests.
///
/// Best-effort: construction is non-blocking. The initial JWT load (from
/// `storage` cache if available, otherwise via HTTP) and all subsequent
/// refreshes run on a background task. Until the first successful load,
/// [`headers`](Self::headers) returns an empty map, so calls proceed without
/// the `x-partner-jwt` header rather than blocking on the JWT fetch.
///
/// The background task uses exponential backoff (capped at 5 minutes) on
/// fetch failures, and refreshes near token expiry on success.
///
/// The task exits when the last `Arc<BreezJwtHeaderProvider>` is dropped: the
/// held `oneshot::Sender` drops, the receiver inside the task resolves with
/// `Err`, and the loop returns.
pub struct BreezJwtHeaderProvider {
    inner: Arc<Inner>,
    _shutdown_tx: oneshot::Sender<()>,
    /// Holds the refresh task's shutdown receiver until [`start`](Self::start)
    /// spawns the task. Taking it makes `start` idempotent.
    pending_rx: Mutex<Option<oneshot::Receiver<()>>>,
}

impl BreezJwtHeaderProvider {
    /// Constructs the provider without starting its background refresh task.
    /// Non-blocking. Call [`start`](Self::start) once storage is known to bind
    /// it and begin refreshing.
    ///
    /// `http_client` is the shared client used for the JWT refresh fetch
    /// (typically supplied from the surrounding [`SdkContext`](crate::SdkContext)).
    pub fn new(api_key: String, http_client: Arc<dyn HttpClient>) -> Arc<Self> {
        let inner = Arc::new(Inner {
            token: RwLock::new(None),
            api_key,
            storage: OnceLock::new(),
            http_client,
        });

        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        Arc::new(Self {
            inner,
            _shutdown_tx: shutdown_tx,
            pending_rx: Mutex::new(Some(shutdown_rx)),
        })
    }

    /// Binds `storage` (for warm-start and cross-restart persistence) and spawns
    /// the refresh task. Idempotent: only the first call starts the task, so a
    /// provider shared across SDK instances is started once. Pass `None` to run
    /// in-memory only. Storage is bound before the task spawns, so its one-shot
    /// `load_cached_token` observes it.
    pub fn start(&self, storage: Option<Arc<dyn Storage>>) {
        let Some(shutdown_rx) = self.pending_rx.lock().unwrap().take() else {
            return;
        };
        if let Some(storage) = storage {
            let _ = self.inner.storage.set(storage);
        }
        spawn_refresh_task(Arc::clone(&self.inner), shutdown_rx);
    }
}

#[macros::async_trait]
impl HeaderProvider for BreezJwtHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        let token_lock = self.inner.token.read().await;
        let Some(cached) = token_lock.as_ref() else {
            return Ok(HashMap::new());
        };

        if is_past_serve_expiry(cached.exp) {
            return Ok(HashMap::new());
        }

        Ok(HashMap::from([(
            PARTNER_ID_HEADER.to_string(),
            cached.token.clone(),
        )]))
    }
}

async fn load_cached_token(inner: &Inner) -> bool {
    let Some(storage) = inner.storage.get() else {
        return false;
    };
    let stored = match storage.get_cached_item(KEY_BREEZ_JWT.to_string()).await {
        Ok(Some(token)) => token,
        Ok(None) => return false,
        Err(err) => {
            warn!("Could not read cached JWT: {err}");
            return false;
        }
    };
    let Some(exp) = jwt_exp(&stored) else {
        return false;
    };
    // Warm-start gates on is_expired (5-min grace), not the tighter serve skew:
    // a loaded token must be comfortably fresh so the spawn task's refresh-sleep
    // shortcut holds. A near-expiry token would hit the 60s fallback (see
    // spawn_refresh_task), installing an about-to-expire token then sleeping
    // before the first fetch, so a restart in the last 5 min drops it and
    // refetches now.
    if is_expired(exp) {
        return false;
    }
    *inner.token.write().await = Some(CachedToken { token: stored, exp });
    true
}

async fn store_token(inner: &Inner, token: String) {
    // Parse the `exp` claim once up front so subsequent expiry checks and
    // refresh-scheduling don't have to re-decode the JWT. An unparseable
    // claim is recorded as `0` (immediate expiry); the refresh loop will
    // retry on the fallback interval rather than tight-looping.
    let exp = jwt_exp(&token).unwrap_or(0);
    *inner.token.write().await = Some(CachedToken {
        token: token.clone(),
        exp,
    });
    if let Some(storage) = inner.storage.get()
        && let Err(err) = storage
            .set_cached_item(KEY_BREEZ_JWT.to_string(), token)
            .await
    {
        warn!("Could not persist JWT: {err}");
    }
}

async fn fetch_new_jwt(api_key: &str, http_client: &Arc<dyn HttpClient>) -> Result<String, String> {
    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), format!("Bearer {api_key}"));
    let res = http_client
        .get(format!("{JWT_BREEZSERVER_URL}/api/jwt"), Some(headers))
        .await
        .map_err(|err| format!("Could not retrieve JWT token: {err}"))?;
    let JwtServerResponse { token } = serde_json::from_str(&res.body)
        .map_err(|err| format!("Could not parse JWT token response: {err}"))?;
    Ok(token)
}

async fn next_refresh_after_success(inner: &Inner) -> Duration {
    let secs = inner
        .token
        .read()
        .await
        .as_ref()
        .map(|c| calculate_expiry(c.exp))
        .filter(|&s| s > 0)
        .unwrap_or(JWT_FALLBACK_INTERVAL_SECS);
    Duration::from_secs(secs)
}

fn backoff_duration(attempt: u32) -> Duration {
    let secs = 1u64
        .checked_shl(attempt)
        .unwrap_or(JWT_BACKOFF_MAX_SECS)
        .min(JWT_BACKOFF_MAX_SECS);
    Duration::from_secs(secs)
}

fn calculate_expiry(exp: u64) -> u64 {
    exp.saturating_sub(Into::<u64>::into(now()).saturating_add(JWT_EXPIRY_GRACE_PERIOD_SECS))
}

/// Returns `true` when the cached `exp` is within (or past) the
/// [`JWT_EXPIRY_GRACE_PERIOD_SECS`] grace window. Drives refresh scheduling, not
/// the serve decision: a token in this window is refreshed early but still
/// served (see [`is_past_serve_expiry`]).
fn is_expired(exp: u64) -> bool {
    calculate_expiry(exp) == 0
}

/// Returns `true` once the cached `exp` is within [`JWT_SERVE_SKEW_SECS`] of (or
/// past) its real expiry, so the token must no longer be served. Unlike
/// [`is_expired`], this keeps serving a still-valid token through the refresh
/// window and withholds it only in the final clock-skew margin.
fn is_past_serve_expiry(exp: u64) -> bool {
    Into::<u64>::into(now()).saturating_add(JWT_SERVE_SKEW_SECS) >= exp
}

#[cfg(test)]
fn is_jwt_expired(token: &str) -> bool {
    let Some(exp) = jwt_exp(token) else {
        return true;
    };
    is_expired(exp)
}

fn jwt_exp(token: &str) -> Option<u64> {
    let payload_b64 = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let payload = std::str::from_utf8(&decoded).ok()?;
    let Jwt { exp } = serde_json::from_str(payload).ok()?;
    Some(exp)
}

fn spawn_refresh_task(inner: Arc<Inner>, mut shutdown_rx: oneshot::Receiver<()>) {
    let span = tracing::Span::current();
    tokio::spawn(
        async move {
            // If a fresh token is already in storage, install it before the
            // first fetch so headers() can start serving it immediately.
            let has_cached = load_cached_token(&inner).await;
            if has_cached {
                let sleep = next_refresh_after_success(&inner).await;
                tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => return,
                    () = tokio::time::sleep(sleep) => {}
                }
            }

            let mut backoff_attempt: u32 = 0;
            loop {
                match fetch_new_jwt(&inner.api_key, &inner.http_client).await {
                    Ok(token) => {
                        store_token(&inner, token).await;
                        backoff_attempt = 0;
                        let sleep = next_refresh_after_success(&inner).await;
                        tokio::select! {
                            biased;
                            _ = &mut shutdown_rx => return,
                            () = tokio::time::sleep(sleep) => {}
                        }
                    }
                    Err(err) => {
                        let sleep = backoff_duration(backoff_attempt);
                        warn!("Could not fetch new JWT (retrying in {:?}): {err}", sleep);
                        backoff_attempt = backoff_attempt.saturating_add(1);
                        tokio::select! {
                            biased;
                            _ = &mut shutdown_rx => return,
                            () = tokio::time::sleep(sleep) => {}
                        }
                    }
                }
            }
        }
        .instrument(span),
    );
}

/// Builds a syntactically valid JWT carrying the given `exp` claim (with a
/// fake signature). Shared by the pure-function and persistence test modules.
#[cfg(test)]
fn make_jwt(exp: u64) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
    format!("{header}.{payload}.fakesignature")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- jwt_exp ---

    #[test]
    fn test_jwt_exp_extracts_value() {
        assert_eq!(jwt_exp(&make_jwt(9_999_999_999)), Some(9_999_999_999));
    }

    #[test]
    fn test_jwt_exp_missing_exp_field() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"user123"}"#);
        assert_eq!(jwt_exp(&format!("h.{payload}.s")), None);
    }

    #[test]
    fn test_jwt_exp_invalid_json() {
        let payload = URL_SAFE_NO_PAD.encode("not json");
        assert_eq!(jwt_exp(&format!("h.{payload}.s")), None);
    }

    // --- is_jwt_expired ---

    #[test]
    fn test_is_jwt_expired_far_past() {
        assert!(is_jwt_expired(&make_jwt(0)));
    }

    #[test]
    fn test_is_jwt_expired_far_future() {
        assert!(!is_jwt_expired(&make_jwt(u64::MAX / 2)));
    }

    #[test]
    fn test_is_jwt_expired_within_grace_period() {
        // Will expire in 2 minutes, which is within the 5-minute grace window.
        let token = make_jwt(u64::from(now()) + 120);
        assert!(is_jwt_expired(&token));
    }

    #[test]
    fn test_is_jwt_expired_malformed_token() {
        assert!(is_jwt_expired("not.a.jwt"));
        assert!(is_jwt_expired("onlyone"));
    }

    // --- is_past_serve_expiry ---

    #[test]
    fn test_is_past_serve_expiry_far_future() {
        // Well beyond the skew margin: servable.
        assert!(!is_past_serve_expiry(u64::from(now()) + 3600));
    }

    #[test]
    fn test_is_past_serve_expiry_within_refresh_grace_still_served() {
        // Expires in 2 minutes: inside the 5-minute refresh lead but outside the
        // 30s serve margin, so it must still be served.
        assert!(!is_past_serve_expiry(u64::from(now()) + 120));
    }

    #[test]
    fn test_is_past_serve_expiry_within_skew_margin() {
        // Expires in 10s: inside the 30s clock-skew margin, so stop serving.
        assert!(is_past_serve_expiry(u64::from(now()) + 10));
    }

    #[test]
    fn test_is_past_serve_expiry_past_and_sentinel() {
        // Already past expiry, and the exp == 0 unparseable-token sentinel.
        assert!(is_past_serve_expiry(u64::from(now()).saturating_sub(60)));
        assert!(is_past_serve_expiry(0));
    }

    // --- backoff_duration ---

    #[test]
    fn test_backoff_grows_exponentially() {
        assert_eq!(backoff_duration(0), Duration::from_secs(1));
        assert_eq!(backoff_duration(1), Duration::from_secs(2));
        assert_eq!(backoff_duration(2), Duration::from_secs(4));
        assert_eq!(backoff_duration(8), Duration::from_secs(256));
    }

    #[test]
    fn test_backoff_caps_at_max() {
        assert_eq!(
            backoff_duration(20),
            Duration::from_secs(JWT_BACKOFF_MAX_SECS)
        );
        assert_eq!(
            backoff_duration(u32::MAX),
            Duration::from_secs(JWT_BACKOFF_MAX_SECS)
        );
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod persistence_tests {
    use super::*;
    use crate::persist::sqlite::SqliteStorage;
    use platform_utils::create_http_client;

    /// Fresh on-disk `SQLite` storage in a unique temp directory.
    fn temp_storage() -> Arc<dyn Storage> {
        let mut dir = std::env::temp_dir();
        dir.push(format!("breez-jwt-test-{}", uuid::Uuid::new_v4()));
        Arc::new(SqliteStorage::new(&dir).expect("create sqlite storage"))
    }

    /// An `Inner` with the given storage (or none). The HTTP client is unused:
    /// these tests exercise only the storage paths, which never fetch.
    fn inner_with_storage(storage: Option<Arc<dyn Storage>>) -> Inner {
        let cell = OnceLock::new();
        if let Some(storage) = storage {
            let _ = cell.set(storage);
        }
        Inner {
            token: RwLock::new(None),
            api_key: "test-key".to_string(),
            storage: cell,
            http_client: create_http_client(Some("jwt-test")),
        }
    }

    #[tokio::test]
    async fn store_then_load_round_trips_a_valid_token() {
        let inner = inner_with_storage(Some(temp_storage()));
        let token = make_jwt(u64::from(now()) + 3600);

        store_token(&inner, token.clone()).await;
        // Clear the in-memory copy so the reload must come from storage: this is
        // the warm-start path that `start(Some(storage))` re-enables.
        *inner.token.write().await = None;

        assert!(load_cached_token(&inner).await);
        assert_eq!(
            inner.token.read().await.as_ref().map(|c| c.token.clone()),
            Some(token)
        );
    }

    #[tokio::test]
    async fn expired_persisted_token_is_not_loaded() {
        let storage = temp_storage();
        storage
            .set_cached_item(KEY_BREEZ_JWT.to_string(), make_jwt(1))
            .await
            .expect("persist token");
        let inner = inner_with_storage(Some(storage));

        assert!(!load_cached_token(&inner).await);
        assert!(inner.token.read().await.is_none());
    }

    #[tokio::test]
    async fn store_without_storage_stays_in_memory_only() {
        // `start(None)` binds no storage: `store_token` updates the live token
        // but persists nothing, so a later provider has no warm-start to load.
        let inner = inner_with_storage(None);
        store_token(&inner, make_jwt(u64::from(now()) + 3600)).await;
        assert!(inner.token.read().await.is_some());
    }
}
