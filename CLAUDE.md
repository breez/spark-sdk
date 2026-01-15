# Breez SDK

Self-custodial Lightning wallet SDK in Rust with bindings for Go, Kotlin, Swift, Python, React Native, C#, WASM, and Flutter.

## Dependencies

```bash
# Debian/Ubuntu
apt-get install -y protobuf-compiler

# macOS
brew install protobuf

# Arch Linux
pacman -S protobuf
```

## Commands

```bash
# Build
make build              # Build workspace (excludes WASM)
make build-wasm         # Build for WASM target

# Test
make cargo-test         # Rust unit tests
make wasm-test          # WASM tests (browser + Node.js)
make itest              # Integration tests (requires Docker)
cargo test <name> -p <package>  # Single test

# Quality
make check              # All checks (fmt, clippy, tests) - use before committing
make fmt-fix            # Fix formatting
make clippy-fix         # Fix clippy issues
```

## Architecture

For crate structure, key abstractions, and data flow, see `.claude/docs/architecture.md`

## Updating SDK Interfaces

For binding update checklist, see `.claude/docs/sdk-interfaces.md`
