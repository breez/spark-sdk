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

**NEW behavior:** The `BitreqHttpClient::post()` (`native.rs`) does **not** set Content-Type automatically via `with_body()`. All current callers manually set Content-Type, so no active bug exists today. But it's fragile - any future caller that forgets to set Content-Type will silently send bare bodies. Consider adding a default Content-Type for POST in the `HttpClient` implementations.

### 3. New HTTP client created per-request in integration tests and faucets

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

### 4. `generate_deposit_address(false)` → `generate_deposit_address(true)` - unrelated change

In `crates/breez-sdk/core/src/sdk/helpers.rs:228`, the argument changed from `false` to `true`. This is commit `23a32d8` ("fix: pass is_static to get_or_create_deposit_address") and appears to be an unrelated bugfix merged into this branch. While likely correct, it should be called out - it changes deposit address generation behavior and has nothing to do with the HTTP migration.

---

## Moderate Issues

### 5. TLS backend change: native-tls → rustls

**OLD:** reqwest was configured with `default-tls` (which is `native-tls` on most platforms, using OpenSSL on Linux, Secure Transport on macOS, SChannel on Windows).

**NEW:** bitreq uses `rustls 0.21.12` (via `async-https` feature). This means:
- Certificate validation now uses `webpki-roots` (Mozilla's bundled CA roots) instead of the system CA store
- Custom CA certificates installed in the system store will **not** be trusted
- Corporate proxies with custom CA certificates will fail with TLS errors
- Older/exotic TLS configurations that worked with OpenSSL may not work with rustls

This is a significant behavioral change that could affect enterprise users.

### 6. `RestClientWrapper` error type mismatch

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

1. **Verify redirect behavior** in bitreq's async Client mode for LNURL flows
2. **Document the TLS backend change** (native-tls → rustls) and assess impact on enterprise users
3. **Consider storing a shared `DefaultHttpClient`** in faucet/mempool test clients instead of creating one per request
4. **Remove unrelated commits** from the PR branch (or document them clearly)
