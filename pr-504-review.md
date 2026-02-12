# PR #504 Review: HTTP Client Migration (reqwest → bitreq) — Updated

## Summary

The PR replaces `reqwest` with `bitreq` on native platforms while keeping `reqwest` for WASM only. A new `platform-utils` crate provides the `HttpClient` trait abstraction. The cleanup commits addressed several issues from the previous review. This updated review covers **only the items that remain unaddressed**.

### What was fixed since the last review

- **Issue #2 (Content-Type headers):** All POST callers now explicitly set `Content-Type: application/json`. The `DefaultLnurlServerClient` has `get_post_headers()` and `get_common_headers()` helpers that set it consistently. Flashnet, faucet, bitcoind, and GraphQL clients all set it explicitly. **Resolved.**
- **Issue #3 (per-request client creation in tests):** `RegtestFaucet`, `MempoolClient`, and `BitcoindFixture` now store `http_client: DefaultHttpClient` as a struct field and reuse it across requests. Connection pooling is preserved. **Resolved.**
- **Issue #4 (unrelated `generate_deposit_address` change):** The commit `23a32d8` is no longer on this branch (removed in force-push). **Resolved.**
- **Minor (unrelated commits):** The branch was rebased/cleaned. Only the LUD-17 fix (`4d98acc`) remains as a separate commit, which is reasonable since it relates to LNURL URL handling. **Mostly resolved.**

---

## Remaining Issues

### 1. Two rustls versions compiled into every binary (rustls 0.21 + 0.23)

**Severity: Moderate**

bitreq 0.3.2 pulls `rustls 0.21.12` + `rustls-webpki 0.101.7` + `tokio-rustls 0.24.1` + `webpki-roots 0.25.4`. Tonic uses `rustls 0.23.x` + `rustls-webpki 0.103.x` + `tokio-rustls 0.26.x` + `webpki-roots 1.0`. Both are compiled into every native binary.

This means:
- Two copies of the TLS handshake state machine
- Two copies of the Mozilla CA root bundle (webpki-roots 0.25 and 1.0) — roughly ~250KB each embedded in the binary
- Two copies of ring's crypto primitives (unless the linker deduplicates)

The whole motivation for this PR is to reduce binary size by dropping reqwest, but shipping two rustls stacks partially negates that benefit. Consider:
- Pushing bitreq upstream to update to rustls 0.23, or
- Using a minimal HTTP client that already depends on rustls 0.23 (e.g., `ureq` 3.x uses rustls 0.23), or
- Accepting this as temporary tech debt with a tracking issue

### 2. `openssl-vendored` feature in Flutter is now dead weight

**Severity: Low (build time / binary size waste)**

`packages/flutter/rust/Cargo.toml` still uses:
```toml
breez-sdk-spark = { path = "...", features = ["openssl-vendored"] }
```

The `openssl-vendored` feature in `crates/breez-sdk/core/Cargo.toml` still exists and compiles vendored OpenSSL. However, since `native-tls` has been removed, nothing in the SDK uses OpenSSL for TLS anymore. The vendored OpenSSL is compiled and linked for no reason — it only adds build time (OpenSSL compilation is slow, especially for cross-compilation) and binary size.

**Fix:** Either remove the `openssl-vendored` feature entirely, or if it's still needed for the `postgres` feature's sqlx dependency, document that clearly and remove it from the Flutter Cargo.toml.

### 3. The lnurl server crate's `sqlx` still uses `tls-native-tls`

**Severity: Informational (not blocking)**

`crates/breez-sdk/lnurl/Cargo.toml`:
```toml
sqlx = { features = ["tls-native-tls"] }
```

This is a separate server binary (not the SDK shipped to integrators), but it means the repo still has a native-tls dependency for Postgres connections. If the goal is to fully remove native-tls from the repo, this should eventually migrate to `tls-rustls` for sqlx. Not blocking for this PR.

### 4. Redirect behavior verification for LNURL flows

**Severity: Needs verification (potentially critical)**

bitreq does follow redirects by default (up to 100 hops), and `client.send_async()` should follow them since it delegates to the same internal logic. However, this has not been explicitly verified for the LNURL callback flow, which relies on redirect-following for:
- LNURL-pay callback URLs that may 301/302
- The input parser's URL resolution for shortened URLs

The code looks correct (bitreq handles redirects at the request level, not client level), but this should be validated with an integration test or manual verification against a known LNURL endpoint that redirects.

### 5. `reqwest` workspace dependency no longer sets a TLS feature

**Severity: Low (WASM-only, but worth noting)**

The workspace `Cargo.toml` defines:
```toml
reqwest = { version = "0.12.23", default-features = false, features = ["json", "http2", "charset", "system-proxy"] }
```

`reqwest` is now only used for WASM (in `platform-utils` and `breez-sdk-common`). On WASM, TLS is handled by the browser, so no TLS feature is needed. However, `http2` and `system-proxy` are unnecessary for WASM (browsers handle both). This is harmless (unused features just won't compile on WASM), but the dependency could be cleaned up to only include what WASM actually uses: `json`.

### 6. Per-request client creation in CLI deposit commands

**Severity: Low (CLI-only)**

In `crates/internal/src/command/deposit.rs`, `DefaultHttpClient::default()` is created per-call in both `get_transaction()` (line 183) and `broadcast_transaction()` (line 208). Each call creates a new connection pool. Since these are CLI commands called infrequently, this has minimal practical impact, but it's inconsistent with the pattern used elsewhere (storing the client in a struct).

### 7. `get_spark_status()` creates a one-off client

**Severity: Low**

In `crates/breez-sdk/core/src/sdk/mod.rs:291`, `get_spark_status()` is a free function (not a method on `BreezSdk`) that creates `DefaultHttpClient::default()` each time it's called. Since it's a standalone status check and not on the hot path, this is fine functionally. Just noting for completeness.

---

## Architecture Assessment

The overall architecture is clean:
- `platform-utils` crate with `HttpClient` trait is a good abstraction
- Platform-specific implementations behind `cfg` gates work well
- `RestClient` trait preserved for backward compatibility with `RestClientWrapper` adapter
- `ServiceConnectivityError` aliased to `HttpError` for API compat
- `DefaultLnurlServerClient` with `get_common_headers()`/`get_post_headers()` helpers is well-structured
- `make_basic_auth_header()` utility avoids repeated base64 encoding logic

The main architectural concern is the dual rustls versions (issue #1), which is a dependency-level problem rather than a code-quality problem.
