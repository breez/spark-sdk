# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Required System Dependencies

| Dependency | Debian/Ubuntu | macOS | Arch |
|------------|---------------|-------|------|
| Protobuf | `apt install protobuf-compiler` | `brew install protobuf` | `pacman -S protobuf` |

## Build Commands

| Command | Purpose |
|---------|---------|
| `make build` | Build workspace (excludes WASM) |
| `make build-release` | Release build with LTO |
| `make build-wasm` | Build for WASM target |

## Testing

To run a single test:
```bash
cargo test <test_name> -p <package>
```

| Command | Purpose |
|---------|---------|
| `make cargo-test` | Rust unit tests |
| `make wasm-test` | WASM tests (browser + Node.js) |
| `make itest` | Integration tests (requires Docker) |
| `make breez-itest` | Breez integration tests (requires faucet credentials) |

## Code Quality

| Command | Purpose |
|---------|---------|
| `make check` | Run all checks (fmt, clippy, tests) - use before committing |
| `make fmt-check` | Check formatting |
| `make fmt-fix` | Fix formatting |
| `make clippy-check` | Run clippy lints (cargo + WASM) |
| `make clippy-fix` | Fix clippy issues |

## Architecture

For crate structure, key abstractions, data flow, and workspace configuration, see `.claude/rules/architecture.md`

## Updating SDK Interfaces

For binding update checklist, see `.claude/rules/bindings.md`
