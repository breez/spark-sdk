use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
/// Cap on the per-domain fetch backoff.
const BACKOFF_MAX_SECS: u64 = 5 * 60;

/// Shared map of allowed domains to their (optional) Breez API key, kept in
/// sync with the DB by the `domains` refresher.
pub type DomainKeys = Arc<RwLock<HashMap<String, Option<String>>>>;

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
        self.0
            .list_domains()
            .await
            .map(|domains| {
                domains
                    .into_iter()
                    .filter_map(|d| d.jwt.map(|jwt| (d.domain, jwt)))
                    .collect()
            })
            .unwrap_or_default()
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
    /// Unix expiry (seconds) parsed from the JWT `exp` claim; `0` if unparseable
    /// (treated as already expired, so it is never served).
    exp: u64,
}

/// Per-domain fetch backoff state.
struct Backoff {
    attempt: u32,
    /// Unix time (seconds) before which no fetch should be attempted.
    next_retry: u64,
}

/// Shared per-domain partner-JWT cache. A background task keeps a token warm for
/// every keyed domain; [`provider_for`](Self::provider_for) hands out a
/// [`DomainJwtHeaderProvider`] bound to one domain that reads this cache.
pub struct JwtCache {
    domains: DomainKeys,
    cache: RwLock<HashMap<String, CachedToken>>,
    http: reqwest::Client,
    jwt_url: String,
    store: Option<Arc<dyn JwtStore>>,
}

impl JwtCache {
    /// Build the cache, warm it from `store`, and spawn the hydrator.
    pub async fn start(domains: DomainKeys, store: Arc<dyn JwtStore>) -> Arc<Self> {
        let cache = Arc::new(Self {
            domains,
            cache: RwLock::new(HashMap::new()),
            http: build_http_client(),
            jwt_url: JWT_URL.to_string(),
            store: Some(store),
        });
        cache.load_persisted().await;
        let hydrator = Arc::clone(&cache);
        tokio::spawn(async move { hydrator.hydrate_loop().await });
        cache
    }

    /// A header provider that emits `x-partner-jwt` for `domain`, reading this
    /// shared cache. Cheap to build per request; it holds no token of its own.
    pub fn provider_for(self: &Arc<Self>, domain: String) -> Arc<DomainJwtHeaderProvider> {
        Arc::new(DomainJwtHeaderProvider {
            cache: Arc::clone(self),
            domain,
        })
    }

    /// The cached token for `domain`, if present and still outside the
    /// clock-skew margin before its expiry. Never performs I/O.
    async fn serve(&self, domain: &str) -> Option<String> {
        let serve_deadline = now_secs().saturating_add(SERVE_SKEW_SECS);
        self.cache
            .read()
            .await
            .get(domain)
            .filter(|t| serve_deadline < t.exp)
            .map(|t| t.token.clone())
    }

    /// Warm the cache from persisted, still-valid tokens for keyed domains.
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
            // Cache the persisted token if not yet expired
            if let Some(exp) = jwt_exp(&token)
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

    async fn hydrate_loop(&self) {
        let mut backoff: HashMap<String, Backoff> = HashMap::new();
        loop {
            self.hydrate_once(&mut backoff).await;
            tokio::time::sleep(HYDRATE_INTERVAL).await;
        }
    }

    /// One hydration pass: drop tokens for domains no longer keyed, then fetch a
    /// token for every keyed domain that is missing or nearing expiry (subject
    /// to per-domain backoff), caching and persisting each success.
    async fn hydrate_once(&self, backoff: &mut HashMap<String, Backoff>) {
        let now = now_secs();
        let domains = self.domains.read().await.clone();
        self.cache
            .write()
            .await
            .retain(|d, _| matches!(domains.get(d), Some(Some(_))));
        backoff.retain(|d, _| matches!(domains.get(d), Some(Some(_))));

        for (domain, key) in &domains {
            let Some(api_key) = key else {
                continue;
            };
            // Skip if a token from the current key isn't yet within the
            // refresh window. A key mismatch counts as stale (domain rotated).
            let fresh = self.cache.read().await.get(domain).is_some_and(|t| {
                t.api_key == *api_key && now.saturating_add(REFRESH_LEAD_SECS) < t.exp
            });
            if fresh {
                continue;
            }
            // Skip if within this domain's backoff window.
            if backoff.get(domain).is_some_and(|b| now < b.next_retry) {
                continue;
            }
            match fetch_jwt(&self.http, &self.jwt_url, api_key).await {
                Ok(token) => {
                    let exp = jwt_exp(&token).unwrap_or(0);
                    self.cache.write().await.insert(
                        domain.clone(),
                        CachedToken {
                            api_key: api_key.clone(),
                            token: token.clone(),
                            exp,
                        },
                    );
                    if let Some(store) = &self.store {
                        store.store(domain, &token).await;
                    }
                    backoff.remove(domain);
                }
                Err(e) => {
                    warn!("could not fetch partner JWT for domain '{domain}': {e}");
                    let attempt = backoff.get(domain).map_or(0, |b| b.attempt);
                    backoff.insert(
                        domain.clone(),
                        Backoff {
                            attempt: attempt.saturating_add(1),
                            next_retry: now.saturating_add(backoff_secs(attempt)),
                        },
                    );
                }
            }
        }
    }
}

/// Emits `x-partner-jwt` for a single domain, reading the shared [`JwtCache`].
/// `headers()` never performs I/O and never returns `Err`, so a missing token
/// yields no header rather than delaying or failing the invoice.
pub struct DomainJwtHeaderProvider {
    cache: Arc<JwtCache>,
    domain: String,
}

#[async_trait::async_trait]
impl HeaderProvider for DomainJwtHeaderProvider {
    async fn headers(&self) -> Result<HashMap<String, String>, HeaderProviderError> {
        Ok(match self.cache.serve(&self.domain).await {
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
fn jwt_exp(token: &str) -> Option<u64> {
    let payload_b64 = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let claims: JwtClaims = serde_json::from_slice(&decoded).ok()?;
    Some(claims.exp)
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

    /// Build a cache without spawning the hydrator, no persistence, pointed at
    /// an unreachable JWT endpoint so any fetch fails fast.
    fn test_cache(keys: &[(&str, Option<&str>)]) -> Arc<JwtCache> {
        test_cache_with_store(keys, None)
    }

    fn test_cache_with_store(
        keys: &[(&str, Option<&str>)],
        store: Option<Arc<dyn JwtStore>>,
    ) -> Arc<JwtCache> {
        let map: HashMap<String, Option<String>> = keys
            .iter()
            .map(|(d, k)| ((*d).to_string(), k.map(str::to_string)))
            .collect();
        Arc::new(JwtCache {
            domains: Arc::new(RwLock::new(map)),
            cache: RwLock::new(HashMap::new()),
            http: reqwest::Client::new(),
            // Port 1 refuses instantly, so a fetch fails without real network.
            jwt_url: "http://127.0.0.1:1/api/jwt".to_string(),
            store,
        })
    }

    async fn seed_cache(cache: &JwtCache, domain: &str, exp: u64) {
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

    /// The `x-partner-jwt` a domain's provider currently emits, if any.
    async fn served(cache: &Arc<JwtCache>, domain: &str) -> Option<String> {
        cache
            .provider_for(domain.to_string())
            .headers()
            .await
            .unwrap()
            .get(PARTNER_JWT_HEADER)
            .cloned()
    }

    /// In-memory [`JwtStore`] keyed by domain.
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
        assert_eq!(jwt_exp(&make_jwt(9_999_999_999)), Some(9_999_999_999));
        assert_eq!(jwt_exp("not.a.jwt"), None);
        assert_eq!(jwt_exp("onlyone"), None);
        // Valid structure, but no `exp` claim.
        let no_exp = format!("h.{}.sig", URL_SAFE_NO_PAD.encode(r#"{"sub":"x"}"#));
        assert_eq!(jwt_exp(&no_exp), None);
    }

    #[test]
    fn test_backoff_secs() {
        assert_eq!(backoff_secs(0), 1);
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(8), 256);
        // Grows past the cap, and never overflows.
        assert_eq!(backoff_secs(20), BACKOFF_MAX_SECS);
        assert_eq!(backoff_secs(u32::MAX), BACKOFF_MAX_SECS);
    }

    #[tokio::test]
    async fn serves_cached_token_for_its_domain() {
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        let exp = now_secs() + 3600;
        seed_cache(&cache, "a.com", exp).await;
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(exp)));
    }

    #[tokio::test]
    async fn token_within_skew_margin_not_served() {
        // Still valid, but within the clock-skew margin of expiry: withheld so
        // the SSP never sees a token it might already consider expired.
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        seed_cache(&cache, "a.com", now_secs() + 10).await;
        assert_eq!(served(&cache, "a.com").await, None);
    }

    #[tokio::test]
    async fn headers_never_fetches_or_errs() {
        // Keyed domain, nothing cached: headers() returns Ok(empty) without any
        // I/O (the endpoint is unreachable; a fetch would hang/fail).
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        let result = cache.provider_for("a.com".to_string()).headers().await;
        assert!(result.is_ok(), "headers() must never return Err");
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_or_expired_yields_empty() {
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        seed_cache(&cache, "a.com", 1).await; // long past
        assert_eq!(served(&cache, "a.com").await, None);
        // A domain with no cache entry at all.
        assert_eq!(served(&cache, "b.com").await, None);
    }

    #[tokio::test]
    async fn providers_are_isolated_by_domain() {
        let cache = test_cache(&[("a.com", Some("key-a")), ("b.com", Some("key-b"))]);
        let exp_a = now_secs() + 3600;
        let exp_b = now_secs() + 7200;
        seed_cache(&cache, "a.com", exp_a).await;
        seed_cache(&cache, "b.com", exp_b).await;
        // Each per-domain provider reads only its own domain's token.
        assert_eq!(served(&cache, "a.com").await, Some(make_jwt(exp_a)));
        assert_eq!(served(&cache, "b.com").await, Some(make_jwt(exp_b)));
    }

    #[tokio::test]
    async fn hydrate_drops_token_when_key_removed() {
        // Domain still present but its key was removed (None): the stale token
        // is dropped so it can't keep attributing to a rotated-out partner.
        let cache = test_cache(&[("a.com", None)]);
        seed_cache(&cache, "a.com", now_secs() + 3600).await;
        cache.hydrate_once(&mut HashMap::new()).await;
        assert!(cache.cache.read().await.get("a.com").is_none());
    }

    #[tokio::test]
    async fn hydrate_keeps_token_and_backs_off_on_fetch_failure() {
        // Keyed domain nearing expiry: the refresh fetch fails (unreachable),
        // but the still-valid old token is kept and a backoff is recorded.
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        let exp = now_secs() + 60; // within the refresh lead
        seed_cache(&cache, "a.com", exp).await;
        let mut backoff = HashMap::new();
        cache.hydrate_once(&mut backoff).await;
        assert_eq!(
            cache.cache.read().await.get("a.com").map(|t| t.exp),
            Some(exp)
        );
        assert!(backoff.contains_key("a.com"));
    }

    #[tokio::test]
    async fn hydrate_refetches_when_key_rotated() {
        // A far-from-expiry token counts as fresh only while the api key it was
        // fetched with still matches the domain's. Re-pointing the domain to a
        // new key makes it stale, so the hydrator attempts a refetch (which
        // fails against the unreachable endpoint, recording a backoff: proof the
        // freshness gate was bypassed).
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        seed_cache(&cache, "a.com", now_secs() + 3600).await; // tagged key-a
        let mut backoff = HashMap::new();

        // Same key: the token stays fresh, so no fetch is attempted.
        cache.hydrate_once(&mut backoff).await;
        assert!(
            !backoff.contains_key("a.com"),
            "a fresh same-key token must not be refetched"
        );

        // Rotate to a different key: the cached token is no longer fresh.
        cache
            .domains
            .write()
            .await
            .insert("a.com".to_string(), Some("key-b".to_string()));
        cache.hydrate_once(&mut backoff).await;
        assert!(
            backoff.contains_key("a.com"),
            "a rotated key must trigger a refetch"
        );
    }

    #[tokio::test]
    async fn hydrate_fetches_caches_serves_and_persists() {
        use axum::{Json, Router, routing::get};

        // A local JWT endpoint returning a valid token.
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
            cache: RwLock::new(HashMap::new()),
            http: reqwest::Client::new(),
            jwt_url: format!("http://{addr}/api/jwt"),
            store: Some(Arc::clone(&store)),
        });

        cache.hydrate_once(&mut HashMap::new()).await;

        // The fetched token is cached + served, and persisted to the store.
        assert_eq!(served(&cache, "a.com").await, Some(token.clone()));
        assert_eq!(store.load_all().await, vec![("a.com".to_string(), token)]);
    }

    #[tokio::test]
    async fn load_persisted_loads_valid_skips_expired_and_keyless() {
        let store = Arc::new(MapStore::default());
        store.store("a.com", &make_jwt(now_secs() + 3600)).await; // valid + keyed
        store.store("b.com", &make_jwt(1)).await; // expired + keyed
        store.store("c.com", &make_jwt(now_secs() + 3600)).await; // valid but keyless
        let cache = test_cache_with_store(
            &[
                ("a.com", Some("key-a")),
                ("b.com", Some("key-b")),
                ("c.com", None),
            ],
            Some(store),
        );

        cache.load_persisted().await;

        let c = cache.cache.read().await;
        assert!(c.contains_key("a.com"));
        assert!(!c.contains_key("b.com"));
        assert!(!c.contains_key("c.com"));
    }

    /// Stand-in for the SSP session-auth provider that the wallet pairs ours
    /// with inside a `CombinedHeaderProvider`.
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

        let cache = test_cache(&[("a.com", Some("key-a"))]);
        let exp = now_secs() + 3600;
        seed_cache(&cache, "a.com", exp).await;

        // Exactly how `ServiceProvider` wires it: [ssp-auth, partner-jwt]. The
        // domain provider carries its own domain, so the merged result has both
        // the auth header and this domain's partner JWT.
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
    /// the real SSP dispatch: a `ServiceProvider` (the SSP client the wallet
    /// uses) built with a `DomainJwtHeaderProvider` and pointed at a local
    /// server must send that domain's `x-partner-jwt` on its request.
    #[tokio::test]
    async fn ssp_request_carries_partner_jwt_for_its_domain() {
        use axum::{Json, Router, http::HeaderMap};
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use spark::session_store::{InMemorySessionStore, Session, SessionStore};
        use spark::ssp::{RetryConfig, ServiceProvider, ServiceProviderConfig};
        use spark_wallet::{DefaultSigner, Network, SparkSignerAdapter};

        // Local server recording the headers of the last inbound request.
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

        // Seed a valid session for the SSP identity so the auth provider returns
        // a cached token without a network handshake, leaving `x-partner-jwt` as
        // the header the request must carry.
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

        // Partner-JWT cache warmed for a.com; the provider is bound to a.com.
        let exp = now_secs() + 3600;
        let cache = test_cache(&[("a.com", Some("key-a"))]);
        seed_cache(&cache, "a.com", exp).await;

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

        // Any SSP query. The response is bogus so the call errors, but the
        // request is sent first; we assert on its headers.
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
