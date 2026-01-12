# PR #534 Review: Seedless Restore

**Author:** dangeross
**Status:** Draft
**Changes:** 37 files, +2,823 −10

## Summary

This PR implements a seedless wallet restore feature using WebAuthn passkeys with PRF (Pseudo-Random Function) extension. The implementation allows users to derive wallet seeds from passkeys without needing to backup mnemonic phrases, with salts stored on Nostr relays for discovery during restore.

---

## Overall Assessment

The implementation is well-structured and follows good design principles. The trait-based approach for `PasskeyPrfProvider` allows for platform-specific implementations while keeping the core logic clean. The code is well-documented with clear module-level documentation and inline comments.

---

## Detailed Review

### 1. Core Implementation (`crates/breez-sdk/core/src/seedless_restore/`)

#### `mod.rs` - Main Orchestration

**Strengths:**
- Clean separation of concerns with the `SeedlessRestore` struct orchestrating between PRF provider and Nostr client
- Good idempotency check before publishing salts (`salt_exists`)
- Comprehensive test coverage with multiple mock implementations

**Concerns:**

```rust
// Line 121-122 in mod.rs
let salt_exists = self.nostr_client.salt_exists(&nostr_keys, &salt).await?;
```
**[QUESTION]** The `salt_exists` check queries all salts and iterates through them. For users with many salts, this could be inefficient. Consider adding a more targeted query in the future if this becomes a performance issue.

---

#### `derivation.rs` - Key Derivation

**Strengths:**
- Correct use of BIP32 derivation for Nostr keypair
- Uses the standard Nostr derivation path `m/44'/1237'/55'/0/0` (account 55 per spec)
- Good error handling for invalid input lengths

**Concerns:**

```rust
// Line 6-7 in derivation.rs
pub const ACCOUNT_MASTER_SALT: &str = "4e594f415354525453414f594e";
```
**[COMMENT]** The magic salt "NYOASTRTSAOYN" (hex-encoded) is well-documented, but consider adding a constant for the decoded string to make the test assertion in `test_account_master_salt_is_valid_hex` clearer:

```rust
// Suggestion:
const ACCOUNT_MASTER_SALT_DECODED: &str = "NYOASTRTSAOYN";
```

---

#### `nostr_client.rs` - Nostr Relay Communication

**Concerns:**

```rust
// Lines 47-48 in nostr_client.rs
client
    .send_event(&event)
    .await
    .map_err(|e| SeedlessRestoreError::SaltPublishFailed(e.to_string()))?;
```
**[QUESTION]** The method doesn't verify that the event was actually persisted to at least one relay. `send_event` might succeed even if all relays rejected the event. Consider checking the result or implementing retry logic.

---

```rust
// Lines 82-86 in nostr_client.rs
let salts: Vec<String> = events
    .into_iter()
    .map(|event| event.content.clone())
    .collect();
```
**[SUGGESTION]** Consider deduplicating salts before returning. If the same salt was published multiple times (e.g., due to network issues), the list will contain duplicates:

```rust
let salts: Vec<String> = events
    .into_iter()
    .map(|event| event.content.clone())
    .collect::<std::collections::HashSet<_>>()
    .into_iter()
    .collect();
```

---

```rust
// Line 99-100 in nostr_client.rs
pub async fn salt_exists(&self, keys: &nostr::Keys, salt: &str) -> Result<bool, SeedlessRestoreError> {
    let salts = self.query_salts(keys).await?;
    Ok(salts.iter().any(|s| s == salt))
}
```
**[MINOR]** This queries all salts just to check if one exists. Not a big concern for now but could be optimized with a filtered query if performance becomes an issue.

---

#### `models.rs` - Nostr Relay Configuration

**Concerns:**

```rust
// Lines 12-19 in models.rs (Default implementation)
relay_urls: vec![
    "wss://relay.nostr.watch".to_string(),
    "wss://relaypag.es".to_string(),
    "wss://monitorlizard.nostr1.com".to_string(),
    "wss://relay.damus.io".to_string(),
    "wss://relay.nostr.band".to_string(),
    "wss://relay.primal.net".to_string(),
],
```
**[QUESTION]** Should the Breez relay (`wss://relay.breez.technology`) be included in the default list? Currently it's only available via `breez_relays()`. For production use, having the Breez relay as a default fallback might improve reliability.

---

### 2. CLI Implementation (`crates/breez-sdk/cli/`)

#### `file_prf.rs` - File-Based PRF Provider

**Strengths:**
- Clear security warnings in documentation
- Uses HMAC-SHA256 which is cryptographically sound for this use case

**Concerns:**

```rust
// Lines 62-65 in file_prf.rs
fs::write(&secret_path, secret).map_err(|e| {
    PasskeyPrfError::Generic(format!("Failed to write secret file: {e}"))
})?;
```
**[SECURITY]** The secret file is written with default permissions. On Unix systems, this means the file could be readable by other users. Consider setting restrictive permissions:

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(&secret_path)?.permissions();
    perms.set_mode(0o600); // Owner read/write only
    fs::set_permissions(&secret_path, perms)?;
}
```

---

#### `main.rs` - CLI Changes

```rust
// Line 93 in main.rs
#[allow(clippy::too_many_lines, clippy::arithmetic_side_effects)]
```
**[MINOR]** The function has grown significantly. Consider extracting the seedless restore logic into a helper function to keep `run_interactive_mode` focused.

---

```rust
// Lines 152-157 in main.rs
let idx: usize = input
    .trim()
    .parse()
    .map_err(|_| anyhow!("Invalid selection"))?;

if idx < 1 || idx > salts.len() {
```
**[MINOR]** The range check `idx < 1 || idx > salts.len()` triggers clippy's `arithmetic_side_effects` warning (hence the allow attribute). Consider using `idx.checked_sub(1).filter(|&i| i < salts.len())` pattern instead.

---

### 3. WASM Bindings (`crates/breez-sdk/wasm/`)

#### `models/passkey_prf_provider.rs`

```rust
// Lines 15-16 in passkey_prf_provider.rs
unsafe impl Send for WasmPasskeyPrfProvider {}
unsafe impl Sync for WasmPasskeyPrfProvider {}
```
**[COMMENT]** The safety comment correctly notes this is safe because WASM is single-threaded. This is a known pattern for WASM FFI.

**Strengths:**
- Excellent TypeScript documentation in the `PASSKEY_PRF_PROVIDER_INTERFACE` constant
- Complete example implementation in the TSDoc

---

#### `seedless_restore.rs`

**Strengths:**
- Clean wrapper around core implementation
- Proper conversion to `JsValue` for errors

---

### 4. Flutter Bindings (`packages/flutter/rust/`)

#### `seedless_restore.rs`

```rust
// Lines 20-23 in seedless_restore.rs
async fn derive_prf_seed(&self, salt: String) -> Result<Vec<u8>, PasskeyPrfError> {
    // DartFnFuture returns the value directly (Dart throws on error)
    Ok((self.derive_prf_seed_fn)(salt).await)
}
```
**[QUESTION]** If Dart throws, does `DartFnFuture` propagate that as a panic or is it caught? The comment suggests Dart exceptions are handled, but there's no explicit error handling here. Consider wrapping in a `catch_unwind` or documenting the expected behavior more clearly.

---

#### `errors.rs`

**Strengths:**
- Proper mirroring of error types for Flutter/Dart compatibility

---

### 5. Documentation (`docs/breez-sdk/src/guide/seedless_restore.md`)

**Strengths:**
- Clear explanation of the two-step derivation process
- Appropriate security warnings about passkey dependency
- Good developer note about platform-specific implementation requirements

**Concerns:**

**[SUGGESTION]** Consider adding:
1. A section on what happens if the passkey is lost (wallet is unrecoverable)
2. Guidance on backup strategies (e.g., registering multiple passkeys)
3. Information about the Nostr relays used and their reliability

---

### 6. Dependencies

```toml
# Cargo.toml
nostr-sdk = { version = "0.43.0", default-features = false }
```
**[COMMENT]** Adding `nostr-sdk` is reasonable for the relay client functionality. The version aligns with the existing `nostr` crate version (0.43.x).

---

## Security Considerations

1. **Salt Visibility**: Salts are published publicly on Nostr. This is by design (per the seedless-restore spec), but users should understand that salts reveal wallet "names" (e.g., "personal", "business").

2. **Relay Trust**: The implementation trusts relays to return all events. A malicious relay could hide salts, preventing restore. Using multiple relays mitigates this.

3. **File-Based PRF (CLI)**: The file-based implementation is for testing only. It should NOT be used in production as the secret is stored in plaintext.

4. **PRF Output Size**: The code correctly validates that PRF output is 32 bytes before use.

---

## Suggested Improvements

### High Priority

1. **[file_prf.rs:62-65]** Set restrictive file permissions (0o600) on the secret file.

2. **[nostr_client.rs:82-86]** Deduplicate salts before returning from `query_salts`.

### Medium Priority

3. **[nostr_client.rs:47-48]** Consider verifying that at least one relay accepted the published event.

4. **[main.rs:93]** Extract seedless restore logic into a helper function.

### Low Priority

5. **[models.rs:12-19]** Consider including Breez relay in default configuration.

6. **[derivation.rs:6-7]** Add decoded constant for clarity.

---

## Questions for Author

1. Is there a plan for handling the case where all Nostr relays are unreachable during restore?

2. Should there be a mechanism to delete/revoke salts from Nostr (noting that Nostr events are generally immutable)?

3. For the Flutter binding, how are Dart exceptions from the callbacks handled on the Rust side?

4. Is there any rate limiting consideration for Nostr queries to avoid being blocked by relays?

---

## Conclusion

This is a well-designed feature implementation. The code is clean, well-documented, and follows the seedless-restore specification. The main areas for improvement are around file permissions in the CLI and salt deduplication in the Nostr client. The security model is clearly documented, and the trait-based architecture allows for proper platform-specific implementations.

**Recommendation:** Approve with minor changes (address file permissions concern).
