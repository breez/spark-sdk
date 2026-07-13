use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use base64::Engine;
use spark::header_provider::{HeaderProvider, HeaderProviderError};
use tokio::sync::RwLock;
use tracing::warn;

use crate::repository::LnurlRepository;

/// Header carrying the Spark partner attribution JWT.
const PARTNER_JWT_HEADER: &str = "x-partner-jwt";
/// Endpoint that issues a partner JWT from a Breez API key.
const JWT_URL: &str = "https://nd1.breez.technology:443/api/jwt";
/// Refresh a cached token once it is within this window of its expiry.
const REFRESH_LEAD_SECS: u64 = 5 * 60;
/// Stop serving a token this many seconds before its real expiry, as a
/// clock-skew guard so we never hand the SSP a token it may already treat as
/// expired.
const SERVE_SKEW_SECS: u64 = 30;
/// How often the hydrator scans the configured domains.
const HYDRATE_INTERVAL: Duration = Duration::from_secs(30);
/// Bound on a single JWT fetch, so a slow endpoint can't stall the hydrator.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap on the per-target fetch backoff.
const BACKOFF_MAX_SECS: u64 = 5 * 60;

/// Shared map of allowed domains to their own Breez API key, or `None` when
/// the domain has none and falls back to the default. Kept in sync with the
/// DB by the `domains` refresher.
pub type DomainKeys = Arc<RwLock<crate::domains::DomainMap>>;

/// Persistence for per-domain JWTs, so restarts and sibling instances start
/// warm instead of re-fetching every token.
#[async_trait::async_trait]
pub trait JwtStore: Send + Sync {
    /// All persisted `(domain, jwt)` pairs, for warming the cache on start.
    async fn load_all(&self) -> Vec<(String, String)>;
    /// Persist a domain's JWT (column-scoped: never touches its api key).
    async fn store(&self, domain: &str, jwt: &str);
}

/// [`JwtStore`] that persists partner JWTs through an [`LnurlRepository`], so the
/// cache survives restarts on the server's own database.
pub struct RepoJwtStore<DB>(pub DB);

#[async_trait::async_trait]
impl<DB> JwtStore for RepoJwtStore<DB>
where
    DB: LnurlRepository + Send + Sync,
{
    async fn load_all(&self) -> Vec<(String, String)> {
        match self.0.list_domains().await {
            Ok(domains) => domains
                .into_iter()
                .filter_map(|d| d.jwt.map(|jwt| (d.domain, jwt)))
                .collect(),
            Err(e) => {
                warn!("could not load persisted partner JWTs: {e}");
                Vec::new()
            }
        }
    }

    async fn store(&self, domain: &str, jwt: &str) {
        if let Err(e) = self.0.set_domain_jwt(domain, jwt).await {
            warn!("could not persist partner JWT for domain '{domain}': {e}");
        }
    }
}

struct CachedToken {
    api_key: String,
    token: String,
    /// Unix expiry (seconds) parsed from the JWT `exp` claim.
    exp: u64,
}

/// Per-target fetch backoff state.
#[derive(Default)]
struct Backoff {
    attempt: u32,
    /// Unix time (seconds) before which no fetch should be attempted.
    next_retry: u64,
}

/// Backoff state carried across hydration passes: one per domain with its own
/// api key, plus one for the default key.
#[derive(Default)]
struct HydrateState {
    per_domain: HashMap<String, Backoff>,
    default: Backoff,
}

/// Shared partner-JWT cache. A background task keeps a token warm for every
/// domain with its own api key (persisted, so restarts start warm) and one for
/// the default key (the catch-all served to domains without their own key).
/// [`provider_for`](Self::provider_for) and [`default_provider`](Self::default_provider)
/// hand out header providers that read this cache.
pub struct JwtCache {
    domains: DomainKeys,
    default_key: Option<String>,
    cache: RwLock<HashMap<String, CachedToken>>,
    default_token: RwLock<Option<CachedToken>>,
    store: Option<Arc<dyn JwtStore>>,
    http: reqwest::Client,
    jwt_url: String,
}

impl JwtCache {
    /// Build the cache, warm it from `store`, and spawn the hydrator.
    pub async fn start(
        domains: DomainKeys,
        default_key: Option<String>,
        store: Arc<dyn JwtStore>,
    ) -> Arc<Self> {
        let cache = Arc::new(Self {
            domains,
            default_key,
            cache: RwLock::new(HashMap::new()),
            default_token: RwLock::new(None),
            store: Some(store),
            http: build_http_client(),
            jwt_url: JWT_URL.to_string(),
        });
        cache.load_persisted().await;
        let hydrator = Arc::clone(&cache);
        tokio::spawn(async move { hydrator.hydrate_loop().await });
        cache
    }

    /// A header provider for `domain`: emits its own partner's `x-partner-jwt`,
    /// falling back to the default key's when its own is not cached.
    pub fn provider_for(self: &Arc<Self>, domain: String) -> Arc<JwtHeaderProvider> {
        Arc::new(JwtHeaderProvider {
            cache: Arc::clone(self),
            target: Target::Domain(domain),
        })
    }

    /// A header provider that always emits the default key's `x-partner-jwt`.
    pub fn default_provider(self: &Arc<Self>) -> Arc<JwtHeaderProvider> {
        Arc::new(JwtHeaderProvider {
            cache: Arc::clone(self),
            target: Target::Default,
        })
    }

    /// Warm the per-domain cache from persisted, still-valid tokens for domains
    /// with their own api key.
    async fn load_persisted(&self) {
        let Some(store) = &self.store else {
            return;
        };
        let now = now_secs();
        let domains = self.domains.read().await.clone();
        for (domain, token) in store.load_all().await {
            let Some(Some(api_key)) = domains.get(&domain) else {
                continue;
            };
            if let Ok(exp) = jwt_exp(&token)
                && now < exp
            {
                self.cache.write().await.insert(
                    domain,
                    CachedToken {
                        api_key: api_key.clone(),
                        token,
                        exp,
                    },
                );
            }
        }
    }

    async fn serve_domain(&self, domain: &str) -> Option<String> {
        let deadline = now_secs().saturating_add(SERVE_SKEW_SECS);
        self.cache
            .read()
            .await
            .get(domain)
            .filter(|t| deadline < t.exp)
            .map(|t| t.token.clone())
    }

    async fn serve_default(&self) -> Option<String> {
        let deadline = now_secs().saturating_add(SERVE_SKEW_SECS);
        self.default_token
            .read()
            .await
            .as_ref()
            .filter(|t| deadline < t.exp)
            .map(|t| t.token.clone())
    }

    /// Serve `domain`'s own token, falling back to the default api key's token
    /// whenever its own is not cached: the domain has no api key, or its token
    /// has not been fetched yet.
    async fn serve_domain_or_default(&self, domain: &str) -> Option<String> {
        match self.serve_domain(domain).await {
            Some(token) => Some(token),
            None => self.serve_default().await,
        }
    }

    async fn hydrate_loop(&self) {
        let mut state = HydrateState::default();
        loop {
            self.hydrate_once(&mut state).await;
            tokio::time::sleep(HYDRATE_INTERVAL).await;
        }
    }

    /// One hydration pass: refresh the token of each domain with its own api key
    /// (persisting each success) and the default api key's token. A token whose
    /// api key no longer matches the domain's counts as stale (domain rotated).
    /// Tokens for domains whose api key was removed are pruned.
    async fn hydrate_once(&self, state: &mut HydrateState) {
        let now = now_secs();
        let domains = self.domains.read().await.clone();
        self.cache
            .write()
            .await
            .retain(|d, _| matches!(domains.get(d), Some(Some(_))));
        state
            .per_domain
            .retain(|d, _| matches!(domains.get(d), Some(Some(_))));

        for (domain, key) in &domains {
            let Some(api_key) = key else {
                continue;
            };
            let current_exp = self.cache.read().await.get(domain).and_then(|t| {
                // Fresh only if from the current key and not within the refresh lead.
                (t.api_key == *api_key).then_some(t.exp)
            });
            let backoff = state.per_domain.entry(domain.clone()).or_default();
            if let Some(token) = self
                .refetch(Some(domain.as_str()), api_key, current_exp, backoff, now)
                .await
            {
                if let Some(store) = &self.store {
                    store.store(domain, &token.token).await;
                }
                self.cache.write().await.insert(domain.clone(), token);
            }
        }

        if let Some(default_key) = &self.default_key {
            let current_exp = self.default_token.read().await.as_ref().map(|t| t.exp);
            if let Some(token) = self
                .refetch(None, default_key, current_exp, &mut state.default, now)
                .await
            {
                *self.default_token.write().await = Some(token);
            }
        }
    }

    /// Fetch `api_key`'s token if the cached one (whose expiry is `current_exp`,
    /// `None` if absent or from a different key) is missing or within the refresh
    /// lead, respecting `backoff`. Returns the new token, or `None` to keep the
    /// existing one and try again later.
    async fn refetch(
        &self,
        domain: Option<&str>,
        api_key: &str,
        current_exp: Option<u64>,
        backoff: &mut Backoff,
        now: u64,
    ) -> Option<CachedToken> {
        let fresh = current_exp.is_some_and(|exp| now.saturating_add(REFRESH_LEAD_SECS) < exp);
        if fresh || now < backoff.next_retry {
            return None;
        }
        let fetched = fetch_jwt(&self.http, &self.jwt_url, api_key)
            .await
            .and_then(|token| {
                jwt_exp(&token)
                    .map(|exp| (token, exp))
                    .map_err(|e| format!("invalid token exp: {e}"))
            });
        match fetched {
            Ok((token, exp)) => {
                *backoff = Backoff::default();
                Some(CachedToken {
                    api_key: api_key.to_string(),
                    token,
                    exp,
                })
            }
            Err(e) => {
                let target = domain.unwrap_or("default key");
                warn!("could not fetch a partner JWT for {target}: {e}");
                backoff.next_retry = now.saturating_add(backoff_secs(backoff.attempt));
                backoff.attempt = backoff.attempt.saturating_add(1);
                None
            }
        }
    }
}

enum Target {
    Domain(String),
    Default,
}

/// Emits `x-partner-jwt` from the shared [`JwtCache`]: for a domain, its own
/// token or the default when its own is not cached; or always the default.
pub struct JwtHeaderProvider {
    cache: Arc<JwtCache>,
    target: Target,
}

#[async_trait::async_trait]
impl HeaderProvider for JwtHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        let token = match &self.target {
            Target::Domain(domain) => self.cache.serve_domain_or_default(domain).await,
            Target::Default => self.cache.serve_default().await,
        };
        Ok(match token {
            Some(token) => HashMap::from([(PARTNER_JWT_HEADER.to_string(), token)]),
            None => HashMap::new(),
        })
    }
}

fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .unwrap_or_default()
}

fn backoff_secs(attempt: u32) -> u64 {
    1u64.checked_shl(attempt)
        .unwrap_or(BACKOFF_MAX_SECS)
        .min(BACKOFF_MAX_SECS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(serde::Deserialize)]
struct JwtResponse {
    token: String,
}

#[derive(serde::Deserialize)]
struct JwtClaims {
    exp: u64,
}

async fn fetch_jwt(http: &reqwest::Client, url: &str, api_key: &str) -> Result<String, String> {
    let resp = http
        .get(url)
        .header("authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("unexpected status {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("body read failed: {e}"))?;
    let parsed: JwtResponse =
        serde_json::from_str(&body).map_err(|e| format!("response parse failed: {e}"))?;
    Ok(parsed.token)
}

/// Parse the `exp` claim from a JWT without verifying the signature.
fn jwt_exp(token: &str) -> anyhow::Result<u64> {
    let payload_b64 = token
        .split('.')
        .nth(1)
        .context("missing JWT payload segment")?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64)?;
    let claims: JwtClaims = serde_json::from_slice(&decoded)?;
    Ok(claims.exp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn make_jwt(exp: u64) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
        format!("{header}.{payload}.sig")
    }

    /// Cache without hydrator or persistence, pointed at an unreachable JWT
    /// endpoint so any fetch fails fast.
    fn test_cache(domains: &[(&str, Option<&str>)], default: Option<&str>) -> Arc<JwtCache> {
        test_cache_with_store(domains, default, None)
    }

    fn test_cache_with_store(
        domains: &[(&str, Option<&str>)],
        default: Option<&str>,
        store: Option<Arc<dyn JwtStore>>,
    ) -> Arc<JwtCache> {
        let map: HashMap<String, Option<String>> = domains
            .iter()
            .map(|(d, k)| ((*d).to_string(), k.map(str::to_string)))
            .collect();
        Arc::new(JwtCache {
            domains: Arc::new(RwLock::new(map)),
            default_key: default.map(str::to_string),
            cache: RwLock::new(HashMap::new()),
            default_token: RwLock::new(None),
            store,
            // Port 1 refuses instantly, so a fetch fails without real network.
            jwt_url: "http://127.0.0.1:1/api/jwt".to_string(),
            http: reqwest::Client::new(),
        })
    }

    async fn seed(cache: &JwtCache, domain: &str, exp: u64) {
        let api_key = cache
            .domains
            .read()
            .await
            .get(domain)
            .cloned()
            .flatten()
            .unwrap_or_default();
        cache.cache.write().await.insert(
            domain.to_string(),
            CachedToken {
                api_key,
                token: make_jwt(exp),
                exp,
            },
        );
    }

    async fn served(cache: &Arc<JwtCache>, domain: &str) -> Option<String> {
        cache
            .provider_for(domain.to_string())
            .headers()
            .await
            .unwrap()
            .get(PARTNER_JWT_HEADER)
            .cloned()
    }

    async fn served_default(cache: &Arc<JwtCache>) -> Option<String> {
        cache
            .default_provider()
            .headers()
            .await
            .unwrap()
            .get(PARTNER_JWT_HEADER)
            .cloned()
    }

    /// In-memory [`JwtStore`] indexed by domain.
    #[derive(Default)]
    struct MapStore(Mutex<HashMap<String, String>>);
    #[async_trait::async_trait]
    impl JwtStore for MapStore {
        async fn load_all(&self) -> Vec<(String, String)> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|(d, j)| (d.clone(), j.clone()))
                .collect()
        }
        async fn store(&self, domain: &str, jwt: &str) {
            self.0
                .lock()
                .unwrap()
                .insert(domain.to_string(), jwt.to_string());
        }
    }

    #[test]
    fn test_jwt_exp() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        assert_eq!(jwt_exp(&make_jwt(9_999_999_999)).unwrap(), 9_999_999_999);
        assert!(jwt_exp("not.a.jwt").is_err());
        assert!(jwt_exp("onlyone").is_err());
        let no_exp = format!("h.{}.sig", URL_SAFE_NO_PAD.encode(r#"{"sub":"x"}"#));
        assert!(jwt_exp(&no_exp).is_err());
    }

    #[test]
    fn test_backoff_secs() {
        assert_eq!(backoff_secs(0), 1);
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(8), 256);
        assert_eq!(backoff_secs(20), BACKOFF_MAX_SECS);
        assert_eq!(backoff_secs(u32::MAX), BACKOFF_MAX_SECS);
    }

    #[tokio::test]
    async fn serves_cached_token_for_its_domain() {
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        let exp = now_secs() + 3600;
        seed(&cache, "a.com", exp).await;
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(exp)));
    }

    #[tokio::test]
    async fn token_within_skew_margin_not_served() {
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        seed(&cache, "a.com", now_secs() + 10).await;
        assert_eq!(served(&cache, "a.com").await, None);
    }

    #[tokio::test]
    async fn headers_never_fetches_or_errs() {
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        let result = cache.provider_for("a.com".to_string()).headers().await;
        assert!(result.is_ok(), "headers() must never return Err");
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_or_expired_yields_empty() {
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        seed(&cache, "a.com", 1).await; // long past
        assert_eq!(served(&cache, "a.com").await, None);
        assert_eq!(served(&cache, "b.com").await, None);
    }

    #[tokio::test]
    async fn providers_are_isolated_by_domain() {
        let cache = test_cache(&[("a.com", Some("key-a")), ("b.com", Some("key-b"))], None);
        let exp_a = now_secs() + 3600;
        let exp_b = now_secs() + 7200;
        seed(&cache, "a.com", exp_a).await;
        seed(&cache, "b.com", exp_b).await;
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(exp_a)));
        assert_eq!(served(&cache, "b.com").await, Some(make_jwt(exp_b)));
    }

    #[tokio::test]
    async fn hydrate_drops_token_when_key_removed() {
        let cache = test_cache(&[("a.com", None)], None);
        seed(&cache, "a.com", now_secs() + 3600).await;
        cache.hydrate_once(&mut HydrateState::default()).await;
        assert!(cache.cache.read().await.get("a.com").is_none());
    }

    #[tokio::test]
    async fn hydrate_keeps_token_and_backs_off_on_fetch_failure() {
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        let exp = now_secs() + 60; // within the refresh lead
        seed(&cache, "a.com", exp).await;
        let mut state = HydrateState::default();
        cache.hydrate_once(&mut state).await;
        assert_eq!(
            cache.cache.read().await.get("a.com").map(|t| t.exp),
            Some(exp)
        );
        assert!(state.per_domain.contains_key("a.com"));
    }

    #[tokio::test]
    async fn hydrate_refetches_when_key_rotated() {
        // A far-from-expiry token counts as fresh only while the api key it was
        // fetched with still matches the domain's. Rotating the domain to a new
        // key makes it stale, so the hydrator attempts a refetch (which fails
        // against the unreachable endpoint, recording a backoff).
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        seed(&cache, "a.com", now_secs() + 3600).await; // tagged key-a
        let mut state = HydrateState::default();
        cache.hydrate_once(&mut state).await;
        assert!(
            state.per_domain.get("a.com").is_none_or(|b| b.attempt == 0),
            "a fresh same-key token must not be refetched"
        );

        cache
            .domains
            .write()
            .await
            .insert("a.com".to_string(), Some("key-b".to_string()));
        cache.hydrate_once(&mut state).await;
        assert!(
            state.per_domain["a.com"].attempt > 0,
            "a rotated key must trigger a refetch"
        );
    }

    #[tokio::test]
    async fn hydrate_refreshes_the_default_key() {
        let cache = test_cache(&[("a.com", None)], Some("key-default"));
        let mut state = HydrateState::default();
        cache.hydrate_once(&mut state).await;
        // The default fetch failed (unreachable), so a backoff is recorded.
        assert!(state.default.attempt > 0);
    }

    #[tokio::test]
    async fn hydrate_fetches_caches_serves_and_persists() {
        use axum::{Json, Router, routing::get};

        let exp = now_secs() + 3600;
        let token = make_jwt(exp);
        let response_token = token.clone();
        let app = Router::new().route(
            "/api/jwt",
            get(move || {
                let token = response_token.clone();
                async move { Json(serde_json::json!({ "token": token })) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let store: Arc<dyn JwtStore> = Arc::new(MapStore::default());
        let cache = Arc::new(JwtCache {
            domains: Arc::new(RwLock::new(HashMap::from([(
                "a.com".to_string(),
                Some("key-a".to_string()),
            )]))),
            default_key: None,
            cache: RwLock::new(HashMap::new()),
            default_token: RwLock::new(None),
            store: Some(Arc::clone(&store)),
            http: reqwest::Client::new(),
            jwt_url: format!("http://{addr}/api/jwt"),
        });

        cache.hydrate_once(&mut HydrateState::default()).await;

        assert_eq!(served(&cache, "a.com").await, Some(token.clone()));
        assert_eq!(store.load_all().await, vec![("a.com".to_string(), token)]);
    }

    #[tokio::test]
    async fn load_persisted_loads_valid_skips_expired_and_without_api_key() {
        let store = Arc::new(MapStore::default());
        store.store("a.com", &make_jwt(now_secs() + 3600)).await; // valid + has key
        store.store("b.com", &make_jwt(1)).await; // expired + has key
        store.store("c.com", &make_jwt(now_secs() + 3600)).await; // valid but no key
        let cache = test_cache_with_store(
            &[
                ("a.com", Some("key-a")),
                ("b.com", Some("key-b")),
                ("c.com", None),
            ],
            None,
            Some(store),
        );

        cache.load_persisted().await;

        let c = cache.cache.read().await;
        assert!(c.contains_key("a.com"));
        assert!(!c.contains_key("b.com"));
        assert!(!c.contains_key("c.com"));
    }

    #[tokio::test]
    async fn default_provider_serves_the_default_slot() {
        let cache = test_cache(&[("a.com", None)], Some("key-default"));
        let exp = now_secs() + 3600;
        *cache.default_token.write().await = Some(CachedToken {
            api_key: "key-default".to_string(),
            token: make_jwt(exp),
            exp,
        });
        assert_eq!(served_default(&cache).await, Some(make_jwt(exp)));
        // A domain with no api key of its own falls back to the default token.
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(exp)));
    }

    #[tokio::test]
    async fn falls_back_to_default_until_own_token_is_cached() {
        let cache = test_cache(&[("a.com", Some("key-a"))], Some("key-default"));
        let default_exp = now_secs() + 3600;
        *cache.default_token.write().await = Some(CachedToken {
            api_key: "key-default".to_string(),
            token: make_jwt(default_exp),
            exp: default_exp,
        });
        // Its own token isn't cached yet (cold start): use the default rather
        // than leave the receive unattributed.
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(default_exp)));

        // Once its own token is cached, that wins over the default.
        let own_exp = now_secs() + 7200;
        seed(&cache, "a.com", own_exp).await;
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(own_exp)));
    }

    struct StubAuth;
    #[async_trait::async_trait]
    impl HeaderProvider for StubAuth {
        async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
            Ok(HashMap::from([(
                "authorization".to_string(),
                "Bearer session".to_string(),
            )]))
        }
    }

    #[tokio::test]
    async fn composes_with_real_combined_header_provider() {
        use spark::header_provider::CombinedHeaderProvider;

        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        let exp = now_secs() + 3600;
        seed(&cache, "a.com", exp).await;

        let combined = CombinedHeaderProvider::new(vec![
            Arc::new(StubAuth) as Arc<dyn HeaderProvider>,
            cache.provider_for("a.com".to_string()) as Arc<dyn HeaderProvider>,
        ]);
        let headers = combined.headers().await.unwrap();
        assert_eq!(
            headers.get("authorization"),
            Some(&"Bearer session".to_string())
        );
        assert_eq!(headers.get(PARTNER_JWT_HEADER), Some(&make_jwt(exp)));
    }

    /// End-to-end check that a domain-bound provider carries attribution through
    /// the real SSP dispatch.
    #[tokio::test]
    async fn ssp_request_carries_partner_jwt_for_its_domain() {
        use axum::{Json, Router, http::HeaderMap};
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use spark::session_store::{InMemorySessionStore, Session, SessionStore};
        use spark::ssp::{RetryConfig, ServiceProvider, ServiceProviderConfig};
        use spark_wallet::{DefaultSigner, Network, SparkSignerAdapter};

        let captured: Arc<Mutex<Option<HeaderMap>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&captured);
        let app = Router::new().fallback(move |headers: HeaderMap| {
            let sink = Arc::clone(&sink);
            async move {
                *sink.lock().unwrap() = Some(headers);
                Json(serde_json::json!({ "data": null }))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let secp = Secp256k1::new();
        let identity =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[1u8; 32]).unwrap());
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::default());
        session_store
            .set_session(
                &identity,
                Session {
                    token: "sess".to_string(),
                    expiration: 9_999_999_999,
                },
            )
            .await
            .unwrap();

        let exp = now_secs() + 3600;
        let cache = test_cache(&[("a.com", Some("key-a"))], None);
        seed(&cache, "a.com", exp).await;

        let signer = Arc::new(DefaultSigner::new(&[1u8; 32], Network::Regtest).unwrap());
        let ssp = ServiceProvider::new(
            ServiceProviderConfig {
                base_url: format!("http://{addr}"),
                schema_endpoint: Some("graphql".to_string()),
                identity_public_key: identity,
                user_agent: None,
                retry_config: RetryConfig::default(),
            },
            Arc::new(SparkSignerAdapter::new(signer)),
            session_store,
            Some(cache.provider_for("a.com".to_string()) as Arc<dyn HeaderProvider>),
        );

        let _ = ssp.get_swap_fee_estimate(1000).await;

        let headers = captured
            .lock()
            .unwrap()
            .take()
            .expect("server received a request");
        assert_eq!(
            headers
                .get(PARTNER_JWT_HEADER)
                .expect("x-partner-jwt present")
                .to_str()
                .unwrap(),
            make_jwt(exp).as_str()
        );
    }
}
