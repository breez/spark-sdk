# PR #504 Review: HTTP Client Migration (reqwest → bitreq)

## Summary

The PR replaces `reqwest` with `bitreq` (a minimal HTTP client from the rust-bitcoin/corepc project) on native platforms while keeping `reqwest` for WASM. A new `platform-utils` crate provides the abstraction layer. The approach is sound architecturally, but there are several concrete HTTP behavior changes that could cause regressions.

---

## Critical Issues

### 1. Missing redirect following on native (bitreq)

**OLD behavior:** `reqwest` follows HTTP redirects automatically (up to 10 hops by default). This is important for LNURL flows where callback URLs may redirect.

**NEW behavior:** `bitreq` follows redirects (up to 100 by default), BUT the `BitreqHttpClient` implementation in `native.rs` calls `response.as_str()` on the raw response. It's unclear whether `client.send_async()` with bitreq's `Client` pool handles redirects the same way as a direct `send()`. The old reqwest followed redirects transparently at the client level. This needs verification - if bitreq's async pooled client doesn't follow redirects, LNURL callbacks and the input parser's URL resolution will break silently (returning redirect HTML/status instead of expected JSON).

### 2. Missing `Content-Type: application/json` header on POST requests

**OLD behavior:** The old `ReqwestLnurlServerClient` used `reqwest`'s `.json(&body)` method, which **automatically sets `Content-Type: application/json`** on every request.

**NEW behavior:** The new `DefaultLnurlServerClient.get_headers()` does add `Content-Type: application/json`, BUT only because the author manually added it. However, in `BitreqHttpClient::post()` (`native.rs:88-103`), the bitreq `with_body()` call does **not** set Content-Type automatically. This means if callers don't pass a Content-Type header, the POST body will be sent without one. Check all callers:

- **`DefaultLnurlServerClient`**: OK - `get_headers()` adds Content-Type.
- **`GraphQLClient.post_query_inner()`**: OK - explicitly inserts Content-Type.
- **`FlashnetClient`**: OK - manually sets Content-Type.
- **Faucet clients**: OK - manually set Content-Type.
- **`broadcast_transaction` in `deposit.rs`**: Sets `Content-Type: text/plain` - OK.
- **LNURL pay/withdraw/auth flows** (`common/src/lnurl/`): These only use GET requests - OK.

So this specific issue is covered, but it's fragile - any future caller that forgets to set Content-Type will silently send bare bodies. Consider adding a default Content-Type for POST in the `HttpClient` implementations.

### 3. Lost HTTP status code in error conversion for GraphQL and Flashnet

**OLD behavior (`GraphQLError::from(reqwest::Error)`):** Extracted the status code with `err.status().map(|s| s.as_u16())`, providing it in the `GraphQLError::Network { code: Some(status_code) }`.

**NEW behavior (`GraphQLError::from(HttpError)`):** Always sets `code: None`:

```rust
impl From<platform_utils::HttpError> for GraphQLError {
    fn from(err: platform_utils::HttpError) -> Self {
        Self::Network {
            reason: err.to_string(),
            code: None,  // <-- REGRESSION
        }
    }
}
```

The same issue exists in `FlashnetError::from(HttpError)` - `code: None` always.

This is a **behavioral regression** because:
- The GraphQL client uses `code: Some(401)` to detect unauthorized responses and trigger re-authentication (`client.rs:132`). If a transport-level 401 is returned as `HttpError` instead of being caught by the success-path status check, the re-auth logic will fail to match.
- The `HttpError::Status { status, body }` variant *does* carry the status code. The conversion should extract it:

```rust
impl From<platform_utils::HttpError> for GraphQLError {
    fn from(err: platform_utils::HttpError) -> Self {
        let code = match &err {
            platform_utils::HttpError::Status { status, .. } => Some(*status),
            _ => None,
        };
        Self::Network {
            reason: err.to_string(),
            code,
        }
    }
}
```

However, note that the new `HttpClient` trait returns `HttpResponse` for all responses (including 4xx/5xx), so transport errors should be rare. The `post_query_inner` method now checks `(400..500).contains(&status_code)` on the response directly. So the practical impact depends on whether bitreq returns transport errors for HTTP error status codes (reqwest has `error_for_status()`, bitreq likely doesn't). **If bitreq always returns Ok with the status in the response (which is what the code assumes), this is a minor issue. But if bitreq can return errors for certain HTTP statuses, the code silently loses the status code.**

### 4. Timeout behavior change

**OLD behavior:** `reqwest` set timeouts per-request with `.timeout(Duration::from_secs(30))`. The timeout covered the entire request lifecycle (connect + send + receive body).

**NEW behavior:** `bitreq`'s `.with_timeout(30)` takes seconds as `u64`. According to bitreq's docs, this is a **total timeout** for the request. However, bitreq's timeout implementation is fundamentally different from reqwest's - it uses either thread-based enforcement (sync) or `tokio::time::timeout_at` (async). The semantics should be similar, but edge cases around slow reads/chunked responses may differ.

**More critically:** The WASM `ReqwestHttpClient` has **no timeout set at all**. The old `ReqwestRestClient` set `.timeout(REQUEST_TIMEOUT)` per-request. The new WASM implementation creates a `reqwest::Client` without timeout and never sets per-request timeout:

```rust
// wasm.rs - no timeout!
let mut req = self.client.get(&url);
// ... no .timeout() call
let response = req.send().await?;
```

This means WASM requests can now hang indefinitely if a server doesn't respond. **This is a regression for WASM targets.**

### 5. New HTTP client created per-request in integration tests and faucets

**OLD behavior:** Faucet clients (`RegtestFaucet`, `MempoolClient`, `BitcoindFixture`) created a `reqwest::Client` once in the constructor and reused it across all requests (connection pooling via hyper's internal pool).

**NEW behavior:** These clients now create a `DefaultHttpClient::default()` **per request**:

```rust
let http_client = DefaultHttpClient::default();
let response = http_client.post(...).await?;
```

Each `DefaultHttpClient::default()` creates a new `BitreqHttpClient` with a new connection pool. This means:
- No connection reuse between requests
- New TCP+TLS handshake for every request
- Higher latency, especially for TLS connections

While this mainly affects test code, the same pattern appears in production-adjacent code. The faucet/mempool clients are worse off.

This is low-severity since it's test-only, but it's a performance regression that could cause flaky tests under load.

### 6. Basic auth helper duplicated 4 times

The function `make_basic_auth_header()` is copy-pasted into:
- `crates/spark-itest/src/faucet.rs`
- `crates/spark-itest/src/fixtures/bitcoind.rs`
- `crates/spark-itest/src/mempool.rs`
- `crates/internal/src/command/deposit.rs`

Two of them use `base64::Engine` directly, two use `base64::write::EncoderStringWriter`. This should be a utility in `platform-utils` or at least shared.

### 7. `generate_deposit_address(false)` → `generate_deposit_address(true)` - unrelated change

In `crates/breez-sdk/core/src/sdk/helpers.rs:228`, the argument changed from `false` to `true`. This is commit `23a32d8` ("fix: pass is_static to get_or_create_deposit_address") and appears to be an unrelated bugfix merged into this branch. While likely correct, it should be called out - it changes deposit address generation behavior and has nothing to do with the HTTP migration.

---

## Moderate Issues

### 8. TLS backend change: native-tls → rustls

**OLD:** reqwest was configured with `default-tls` (which is `native-tls` on most platforms, using OpenSSL on Linux, Secure Transport on macOS, SChannel on Windows).

**NEW:** bitreq uses `rustls 0.21.12` (via `async-https` feature). This means:
- Certificate validation now uses `webpki-roots` (Mozilla's bundled CA roots) instead of the system CA store
- Custom CA certificates installed in the system store will **not** be trusted
- Corporate proxies with custom CA certificates will fail with TLS errors
- Older/exotic TLS configurations that worked with OpenSSL may not work with rustls

This is a significant behavioral change that could affect enterprise users.

### 9. Error message detail loss in bitreq → HttpError conversion

**OLD:** `reqwest::Error` conversion walks the error chain with `source()` to build a detailed message:

```rust
let mut err_str = err.to_string();
while let Some(src) = walk.source() {
    err_str.push_str(format!(" : {src}").as_str());
    walk = src;
}
```

**NEW (native):** `bitreq::Error` conversion uses `format!("{err:?}")` (Debug format) and maps most errors to a generic `Other` variant:

```rust
match err {
    bitreq::Error::IoError(_) => Self::Connect(err_str),
    bitreq::Error::InvalidUtf8InBody(_) => Self::Decode(err_str),
    bitreq::Error::Other(msg) => Self::Other(msg.to_string()),
    _ => Self::Other(err_str),  // catch-all loses error classification
}
```

This means:
- **Timeout errors** from bitreq will map to `HttpError::Other` instead of `HttpError::Timeout`
- **Redirect errors** will map to `HttpError::Other` instead of `HttpError::Redirect`
- Any code that pattern-matches on specific `HttpError` variants (like `Timeout`) won't catch bitreq timeouts

### 10. `check_username_available` now sends Content-Type and User-Agent headers on GET

The old `ReqwestLnurlServerClient.check_username_available()` made a bare `self.client.get(url).send().await` (no extra headers on the GET, only default headers from the client builder: Authorization + User-Agent).

The new version calls `self.get_headers()` which adds `Content-Type: application/json` and `User-Agent: breez-sdk-spark` on every request including GETs. Sending `Content-Type` on a GET with no body is unusual and could confuse some servers or CDN caches. It's technically harmless per HTTP spec but a behavioral change.

### 11. `RestClientWrapper` error type mismatch

The `RestClientWrapper` bridges `RestClient` (returning `ServiceConnectivityError`) to `HttpClient` (returning `HttpError`). Since `ServiceConnectivityError` is now just a type alias for `HttpError`, this works. But the `?` operator in:

```rust
Ok(self.inner.get_request(url, headers).await?.into())
```

converts `ServiceConnectivityError` (= `HttpError`) through the `?` into `HttpError`, which is an identity conversion. This is correct but worth noting: if someone provides a custom `RestClient` that returns the old-style error variants, they'll pass through unchanged.

---

## Minor/Cosmetic Issues

- The PR includes several unrelated commits (Kotlin docs fix, Flutter fixes, deposit address fix) that should ideally be separate PRs
- `bitreq 0.3.2` uses `rustls 0.21.12` which is older than the `rustls 0.23.31` used by the rest of the project (for gRPC/tonic). This means two versions of rustls are compiled in, increasing binary size
- The `REQUEST_TIMEOUT` constant is `u64` in platform-utils but was `Duration` in the old code - not a bug but less type-safe
- The connection pool size of 10 (`DEFAULT_POOL_CAPACITY`) is hardcoded and smaller than reqwest's default (which uses hyper's pool with no hard cap)

---

## Recommendations

1. **Fix the WASM timeout regression** - add `.timeout(Duration::from_secs(REQUEST_TIMEOUT))` to the WASM `ReqwestHttpClient` methods
2. **Fix the error conversion** in `GraphQLError::from(HttpError)` and `FlashnetError::from(HttpError)` to extract status codes from `HttpError::Status`
3. **Improve the bitreq→HttpError mapping** to properly classify timeouts, redirects, etc.
4. **Verify redirect behavior** in bitreq's async Client mode for LNURL flows
5. **Document the TLS backend change** (native-tls → rustls) and assess impact on enterprise users
6. **Extract `make_basic_auth_header`** into a shared utility
7. **Remove unrelated commits** from the PR branch (or document them clearly)
