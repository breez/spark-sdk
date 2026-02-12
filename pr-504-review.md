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

### 5. TLS backend change: native-tls → rustls (all native platforms)

The PR **removes all TLS feature flags** (`default-tls`, `rustls-tls`, `native-tls`) from every crate in the workspace. The `native-tls` crate is entirely removed from `Cargo.lock`. This is a hard switch with no opt-out.

#### Before (per platform, per subsystem)

| Platform | HTTP (REST) | gRPC (tonic) | Cert Source |
|----------|-------------|--------------|-------------|
| **Linux** | OpenSSL via `native-tls` | rustls 0.23 | System CA store (HTTP) / webpki-roots (gRPC) |
| **macOS** | Secure Transport via `native-tls` | rustls 0.23 | System Keychain (HTTP) / webpki-roots (gRPC) |
| **Windows** | SChannel via `native-tls` | rustls 0.23 | System cert store (HTTP) / webpki-roots (gRPC) |
| **iOS (Swift UniFFI)** | Secure Transport via `native-tls` | rustls 0.23 | System (HTTP) / webpki-roots (gRPC) |
| **Android (Kotlin UniFFI)** | OpenSSL via `native-tls` | rustls 0.23 | System (HTTP) / webpki-roots (gRPC) |
| **Flutter (iOS/Android)** | OpenSSL **vendored** via `native-tls` + `openssl-vendored` | rustls 0.23 | Vendored OpenSSL (HTTP) / webpki-roots (gRPC) |
| **Go UniFFI** | OpenSSL via `native-tls` | rustls 0.23 | System (HTTP) / webpki-roots (gRPC) |
| **Python UniFFI** | OpenSSL via `native-tls` | rustls 0.23 | System (HTTP) / webpki-roots (gRPC) |
| **React Native** | OpenSSL via `native-tls` | rustls 0.23 | System (HTTP) / webpki-roots (gRPC) |
| **WASM (browser)** | Browser Fetch API | gRPC-web via browser | Browser CA store |

The feature chain was: `default = ["default-tls"]` → `reqwest/default-tls` → `hyper-tls` → `native-tls` crate → platform TLS library.

Cargo.lock confirmed: `reqwest` depended on `hyper-tls`, `native-tls`, `tokio-native-tls`, `openssl-sys` (Linux), `security-framework` (macOS/iOS), `schannel` (Windows).

#### After (per platform, per subsystem)

| Platform | HTTP (REST) | gRPC (tonic) | Cert Source |
|----------|-------------|--------------|-------------|
| **Linux** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **macOS** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **Windows** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **iOS (Swift UniFFI)** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **Android (Kotlin UniFFI)** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **Flutter (iOS/Android)** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **Go UniFFI** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **Python UniFFI** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **React Native** | **rustls 0.21** via bitreq | rustls 0.23 | **webpki-roots 0.25** bundled (HTTP) / webpki-roots 1.0 (gRPC) |
| **WASM (browser)** | Browser Fetch API (unchanged) | gRPC-web via browser (unchanged) | Browser CA store (unchanged) |

bitreq 0.3.2 depends on: `rustls 0.21.12` + `rustls-webpki 0.101.7` + `tokio-rustls 0.24.1` + `webpki-roots 0.25.4`.

#### Key consequences

1. **System CA certificates are no longer used for HTTP on any native platform.** All certificate validation uses bundled Mozilla roots (`webpki-roots 0.25.4`). Custom/corporate CA certs in the system store will NOT be trusted for REST calls. This affects all UniFFI targets (iOS, Android, Go, Kotlin, Python, Swift, React Native) and Flutter.

2. **Two rustls versions compiled into every binary.** bitreq pulls rustls 0.21.12, tonic uses rustls 0.23.31. Both are compiled in, increasing binary size. They also pull different webpki-roots versions (0.25 vs 1.0).

3. **The `openssl-vendored` feature in Flutter is now dead weight.** The feature still compiles vendored OpenSSL (`openssl-sys` + `openssl-src`), but nothing uses it for TLS anymore since `native-tls` is gone. It just adds build time and binary size for no benefit.

4. **No opt-out.** The old `rustls-tls` / `native-tls` / `default-tls` feature flags are deleted from all crates. Integrators who were explicitly selecting `native-tls` will get a build error.

5. **iOS note:** Before, Secure Transport was used which integrates with iOS App Transport Security (ATS) and the iOS keychain for client certificates. After, rustls does not integrate with ATS or the keychain.

6. **The lnurl crate's `sqlx` still uses `tls-native-tls`** (`sqlx = { features = ["tls-native-tls"] }`), so that crate still depends on native-tls/OpenSSL for its Postgres connections. This is a separate binary (lnurl server) not the SDK itself, but it shows the migration is incomplete across the repo.

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
2. **Assess TLS migration impact** - the switch from system TLS to bundled rustls affects all native platforms. Consider:
   - Whether iOS ATS compliance is affected
   - Whether any integrators rely on system CA certificates or client certificates
   - Whether the `openssl-vendored` feature in Flutter should be removed (it's now dead weight)
   - Whether the dual rustls versions (0.21 + 0.23) are acceptable for binary size
3. **Consider storing a shared `DefaultHttpClient`** in faucet/mempool test clients instead of creating one per request
4. **Remove unrelated commits** from the PR branch (or document them clearly)
