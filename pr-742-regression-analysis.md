# PR #742 Regression Analysis: Replacing `url` crate with `bitreq::Url`

## Summary

PR #742 removes the `url` crate and replaces all `url::Url` usage with `bitreq::Url` (upgraded from 0.3.2 to 0.3.4). While this reduces dependencies, `bitreq::Url` is a minimal URL parser that lacks several normalization behaviors of `url::Url` (which implements the WHATWG URL Standard). This introduces behavioral regressions.

---

## Regression #1: Host not lowercased (CRITICAL)

`url::Url` normalizes the host to lowercase per the WHATWG URL spec. `bitreq::Url` only lowercases the **scheme**, preserving original host casing.

For `https://DOMAIN.COM/path`:
- `url::Url::domain()` → `"domain.com"` (lowercased)
- `bitreq::Url::base_url()` → `"DOMAIN.COM"` (preserved as-is)

### Affected code

| File | Old code | New code | Impact |
|------|----------|----------|--------|
| `lnurl/auth.rs:118` | `url.domain()` | `url.base_url()` | LNURL-auth HMAC key derivation uses domain. Different casing → different key → **auth failure** |
| `lnurl/auth.rs:125` | `url.domain()` | `url.base_url()` | Same HMAC derivation issue in `get_derivation_path` |
| `lnurl/pay.rs:389,395` | `req_url.domain()` / `action_res_url.domain()` | `.base_url()` | Domain comparison for success action validation. Case mismatch → **false rejection of valid success actions** |
| `input/parser/mod.rs` (resolve_lnurl) | `url.host()` | `url.base_url()` | Domain stored in `LnurlPayRequestDetails` shown to users — would display unnormalized casing |
| `input/parser/mod.rs` (parse_external_input) | `url.host_str()` | `url.base_url()` | Domain extracted for external parser results |

### Fix

Add `.to_ascii_lowercase()` wherever `base_url()` is used for domain extraction/comparison.

---

## Regression #2: Onion detection fails when URL has explicit port (MODERATE)

In the new pre-parse scheme rewriting (`input/parser/mod.rs` ~line 297-316), the host is extracted from the **raw URL string** before parsing:

```rust
let host = after_scheme.split('/').next().unwrap_or("").split('?').next().unwrap_or("");
```

For `lnurlp://example.onion:8080/path`, this gives `"example.onion:8080"` (includes port).

Then `has_extension("example.onion:8080", "onion")` returns **FALSE** because `std::path::Path::extension()` sees the extension as `"onion:8080"`, not `"onion"`.

### Result

The URL is incorrectly rewritten to `https://example.onion:8080/path` instead of `http://example.onion:8080/path`. The subsequent scheme check then hits `has_extension(&host, "onion")` on the **parsed** host (which correctly strips the port), returning `Err(HttpsSchemeWithOnionDomain)`.

**Onion domains with explicit ports and lnurl prefixes would fail entirely.**

The original code parsed with `url::Url` first (which properly separates host from port), so onion detection always worked.

### Fix

Strip the port before the onion check:

```rust
let host = after_scheme
    .split('/').next().unwrap_or("")
    .split('?').next().unwrap_or("")
    .split(':').next().unwrap_or("");  // strip port
```

---

## Regression #3: `domain()` vs `base_url()` for IP addresses (LOW)

`url::Url::domain()` returns `None` for IP-based URLs (only returns `Some` for actual domain names). `bitreq::Url::base_url()` always returns the host string regardless.

For `https://127.0.0.1:8080/path`:
- `url::Url::domain()` → `None`
- `bitreq::Url::base_url()` → `"127.0.0.1"`

### Affected code

- `lnurl/auth.rs`: `validate_request` and `get_derivation_path` — IP-based auth URLs would now succeed where they previously failed with `MissingDomain`
- `lnurl/pay.rs`: `UrlSuccessActionData::validate` — IP-based callback/success-action URLs would now succeed

This is a **behavioral change** rather than a strict regression (arguably an improvement), but worth noting for consistency.

---

## Regression #4: Non-ASCII URL rejection (LOW)

`bitreq::Url::parse()` rejects any non-ASCII character:

```rust
for c in url_str.chars() {
    if !c.is_ascii() || c.is_ascii_control() {
        return Err(ParseError::InvalidCharacter(c));
    }
}
```

`url::Url` handles IDNA/punycode and percent-encoded Unicode. Lightning addresses or LNURL endpoints with internationalized domain names would fail with `bitreq::Url`.

---

## Regression #5: Unknown schemes return `Ok(None)` instead of `Err(UnknownScheme)` (LOW)

`bitreq::Url::parse()` fails with `MissingPort` for unknown schemes without explicit ports. The parser catches this with `let Ok(parsed_url) = bitreq::Url::parse(&input) else { return Ok(None); }`.

Previously, `url::Url::parse()` would succeed for any scheme, and the match arm `&_ => return Err(LnurlError::UnknownScheme)` would fire.

**Behavioral change**: unknown-scheme URLs now silently return `None` (not recognized) instead of returning an error.

---

## Minor difference: URL serialization

`url::Url` adds a trailing slash for root URLs (`https://domain.com` → `https://domain.com/`). `bitreq::Url` preserves the URL as-is. This affects `url.to_string()` output stored in struct fields and used for HTTP requests, but is unlikely to cause practical issues since most LNURL endpoints have paths.
