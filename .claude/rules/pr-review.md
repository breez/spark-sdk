# PR Review Guidelines for Breez SDK

## Overview

This document defines the review criteria for pull requests in the Breez SDK repository.
Reviews should be thorough but efficient, focusing on actionable feedback.

---

## Review Categories

### 1. Code Quality (HIGH Priority)

**Formatting & Linting**
- All code must pass `make fmt-check`
- All code must pass `make clippy-check` and `make wasm-clippy-check`
- Clippy warnings require `#[allow()]` with documented justification

**Rust Conventions**
- Use `Result<T, E>` for fallible operations (no unwrap/expect in SDK code)
- Prefer `?` operator over manual error handling
- Follow existing naming patterns in the codebase
- Avoid large enum variants (clippy `large_enum_variant`)

**Documentation**
- Public structs/enums/functions must have `///` doc comments
- Doc examples must compile (verified by CI)
- Comments should explain "why", not "what"

### 2. API & Bindings (HIGH Priority)

When public API changes, verify ALL binding files are updated:
1. `crates/breez-sdk/core/src/models.rs` - UniFFI attributes
2. `crates/breez-sdk/wasm/src/models.rs` - WASM exports
3. `crates/breez-sdk/wasm/src/sdk.rs` - WASM interface
4. `packages/flutter/rust/src/models.rs` - Flutter structs
5. `packages/flutter/rust/src/sdk.rs` - Flutter interface

**Breaking Changes**
- Require explicit version bump consideration
- Document migration path in PR description

### 3. Testing (HIGH Priority)

**Requirements**
- New features must include unit tests
- Bug fixes must include regression tests
- Tests must be deterministic (no flaky tests)

**Verification Commands**
```bash
make cargo-test      # Rust tests
make wasm-test       # WASM tests (browser + Node.js)
make itest           # Spark integration tests
make breez-itest     # Breez integration tests
```

### 4. Security (CRITICAL Priority)

**Key Management**
- Private keys never logged or serialized unnecessarily
- Key derivation follows BIP standards
- Randomness uses secure sources (`rand` crate)
- No keys in error messages

**Common Vulnerabilities**
- Input validation at system boundaries
- No command injection in shell operations
- Proper error handling (don't leak sensitive info)

**Cryptographic Operations**
- Use checked arithmetic (`checked_add`, `checked_mul`)
- Schnorr signing must use `aux_rand`
- Validate all external inputs

### 5. Performance (MEDIUM Priority)

- No unnecessary allocations in hot paths
- Async operations must not block
- Database queries should use indexes
- Network calls must have timeouts
- Avoid excessive cloning of large structures

### 6. Architecture (MEDIUM Priority)

**Abstractions**
- `Storage` trait changes must maintain backward compatibility
- `Signer` operations shouldn't modify wallet state
- Event emissions for async notifications

**Platform-Specific Code**
- WASM: Use `#[cfg(target_family = "wasm")]`
- Time operations: Use `web_time` crate for WASM
- File I/O: Platform-appropriate storage

---

## Commit Standards

**Format**: Conventional Commits
```
<type>(<scope>): <description> (#issue)

Types: feat, fix, docs, refactor, perf, test, chore
```

**Requirements**
- Each commit is logically complete
- No bundling unrelated changes
- Reference related issues
- Linear history (no merge commits)

---

## Review Output Format

Structure your review as follows:

### Summary
Brief description of what the PR does.

### Issues Found
List by severity:
- **CRITICAL**: Must fix before merge (security, data loss)
- **HIGH**: Should fix before merge (bugs, missing tests)
- **MEDIUM**: Recommend fixing (performance, style)
- **LOW**: Minor suggestions (optional improvements)

### Questions
Areas needing clarification from the author.

### Recommendation
- **APPROVE**: Ready to merge
- **REQUEST CHANGES**: Issues must be addressed
- **COMMENT**: Feedback only, no blocking issues

---

## Quick Checklist

```
Code Quality:
[ ] Formatting passes (make fmt-check)
[ ] Linting passes (make clippy-check + wasm-clippy-check)
[ ] Public API documented
[ ] No unwrap/panic in SDK code

Testing:
[ ] Unit tests for new features
[ ] Regression tests for bug fixes
[ ] CI passes (all green)

API Changes:
[ ] All 5 binding files updated
[ ] No breaking changes OR version bump planned
[ ] WASM builds (make build-wasm)

Security:
[ ] No hardcoded secrets
[ ] Input validation present
[ ] Keys handled securely

Commits:
[ ] Conventional commit format
[ ] Issues referenced
[ ] Clean history
```

---

## Anti-Patterns to Flag

| Pattern | Why It's Bad |
|---------|--------------|
| `unwrap()` / `expect()` in SDK | Panics in library code |
| Blocking in async context | Deadlocks, poor performance |
| Hardcoded magic numbers | Poor maintainability |
| Missing error context | Hard to debug |
| Large enum variants | Memory inefficiency |
| Unchecked arithmetic | Potential overflow |
