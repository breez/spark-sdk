# PR Review Guidelines for Breez SDK

## Overview

This document defines the review criteria for pull requests in the Breez SDK repository.
Reviews should validate both **implementation correctness** and **design decisions**.
A good review confirms the approach is right, not just that the code works.

---

## Review Categories

### 0. Design & Rationale (CRITICAL Priority)

Before diving into code, evaluate the design:

**Problem Understanding**
- What problem does this PR solve? (UX, performance, correctness, etc.)
- Is the problem clearly stated in the PR description?
- Does the solution match the problem scope?

**Alternative Approaches**
- Were other designs considered? (Ask if not mentioned)
- Trade-offs between approaches (e.g., recursive vs flat structures, joins vs denormalization)
- Why was this approach chosen over alternatives?

**Impact Assessment**
- Schema/API changes: backward compatibility?
- Data consistency: what happens on edge cases (deletion, partial updates)?
- Migration path for existing users/data

**Future Extensibility**
- Does this generalize or is it special-cased?
- How does this scale if requirements expand?
- Will this design accommodate future needs without major refactoring?

**Questions to Ask**
- "Why this approach over X?"
- "What happens if Y is deleted/fails?"
- "Does this need to support Z in the future?"

---

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
Brief description of what the PR does and the problem it solves.

### Design Analysis
- **Rationale**: Why is this change needed? Does the PR description explain the "why"?
- **Approach**: Is this the right solution? What alternatives exist?
- **Trade-offs**: What are the costs of this approach (complexity, performance, maintenance)?
- **Extensibility**: Does this generalize well or is it narrowly scoped?

### Issues Found
List by severity:
- **CRITICAL**: Must fix before merge (security, data loss, design flaws)
- **HIGH**: Should fix before merge (bugs, missing tests, unclear rationale)
- **MEDIUM**: Recommend fixing (performance, style)
- **LOW**: Minor suggestions (optional improvements)

### Questions
Areas needing clarification from the author. Examples:
- "Why was X chosen over Y?"
- "What happens when Z fails?"
- "Will this need to support W in the future?"

### Recommendation
- **APPROVE**: Design is sound, implementation is correct
- **REQUEST CHANGES**: Design or implementation issues must be addressed
- **COMMENT**: Feedback only, no blocking issues

---

## Quick Checklist

```
Design & Rationale:
[ ] Problem clearly stated in PR description
[ ] Approach justified (why this over alternatives?)
[ ] Backward compatibility considered
[ ] Edge cases handled (deletion, failures, partial states)
[ ] Future extensibility considered

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
[ ] Schema migrations included if needed

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
