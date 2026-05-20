use std::collections::HashMap;
use std::sync::Arc;

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
    storage: Option<Arc<dyn Storage>>,
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
}

impl BreezJwtHeaderProvider {
    /// Constructs the provider and spawns its background refresh task.
    /// Does not block on the initial JWT load.
    ///
    /// `http_client` is the shared client used for the JWT refresh fetch
    /// (typically supplied from the surrounding [`SdkContext`](crate::SdkContext)).
    pub fn new(
        api_key: String,
        storage: Option<Arc<dyn Storage>>,
        http_client: Arc<dyn HttpClient>,
    ) -> Arc<Self> {
        let inner = Arc::new(Inner {
            token: RwLock::new(None),
            api_key,
            storage,
            http_client,
        });

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        spawn_refresh_task(Arc::clone(&inner), shutdown_rx);

        Arc::new(Self {
            inner,
            _shutdown_tx: shutdown_tx,
        })
    }
}

#[macros::async_trait]
impl HeaderProvider for BreezJwtHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        let token_lock = self.inner.token.read().await;
        let Some(cached) = token_lock.as_ref() else {
            return Ok(HashMap::new());
        };

        if is_expired(cached.exp) {
            return Ok(HashMap::new());
        }

        Ok(HashMap::from([(
            PARTNER_ID_HEADER.to_string(),
            cached.token.clone(),
        )]))
    }
}

async fn load_cached_token(inner: &Inner) -> bool {
    let Some(storage) = &inner.storage else {
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
    if let Some(storage) = &inner.storage
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
/// [`JWT_EXPIRY_GRACE_PERIOD_SECS`] grace window.
fn is_expired(exp: u64) -> bool {
    calculate_expiry(exp) == 0
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(exp: u64) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
        format!("{header}.{payload}.fakesignature")
    }

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
