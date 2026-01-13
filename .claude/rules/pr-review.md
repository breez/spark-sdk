# PR Review Guidelines

See `CLAUDE.md` for build commands, test commands, and binding file locations.

## Design (CRITICAL)

Before reviewing code, evaluate the approach:

- Is the problem clearly stated in the PR description?
- How will app developers use this API? (UX-first)
- Why this approach over alternatives?
- Backward compatibility impact?
- Edge cases: what happens on deletion/failure/partial state?

Prefer semantic types over generic ones:
- Bad: `Vec<RelatedPayment>`
- Good: `ConversionInfo { sent: Payment, received: Payment }`

## Security (CRITICAL)

- No keys in logs or error messages
- Checked arithmetic for crypto ops (`checked_add`, `checked_mul`)
- Input validation at boundaries
- Schnorr signing must use `aux_rand`

## Code Quality

- No `unwrap()`/`expect()` in SDK code
- Public API has `///` doc comments
- Clippy clean (or `#[allow()]` with justification)

## Before Approving
```bash
make check       # fmt, clippy, tests
make build-wasm  # verify WASM builds
```

Verify all binding files updated (see CLAUDE.md → "Updating SDK Interfaces").

## Anti-Patterns

| Pattern | Issue |
|---------|-------|
| `unwrap()` in SDK | Panics in library code |
| Blocking in async | Deadlocks |
| Large enum variants | Memory inefficiency |
| Unchecked arithmetic | Overflow risk |
