# CI Reference

What CI validates - PR reviewers can skip commenting on issues CI catches.

## Jobs Summary

| Job | Runner | What It Validates |
|-----|--------|-------------------|
| `clippy` | ubuntu | Rust lint warnings (pedantic, suspicious, complexity, perf) |
| `fmt` | ubuntu | Code formatting via rustfmt |
| `test` | ubuntu | Rust unit tests (excludes itest, wasm) |
| `itest` | ubuntu | Integration tests with Docker (bitcoind, postgres, spark-so) |
| `breez-test` | ubuntu | Live Breez service tests with faucet |
| `wasm-clippy` | ubuntu | Clippy for wasm32-unknown-unknown target |
| `wasm-test` | ubuntu | WASM tests in browser (Firefox) and Node.js |
| `flutter` | ubuntu | Flutter binding generation and build |
| `docs-*` | varies | Doc snippet validation per language |

## What Each Job Catches

### clippy
- All clippy warnings treated as errors (`-D warnings`)
- Runs on all targets and tests
- Excludes WASM packages (separate job)
- Also runs on out-of-workspace packages (`crates/breez-sdk/lnurl`)

### fmt
- Formatting violations via `cargo fmt --check`
- Covers workspace and out-of-workspace packages

### test
- Unit test failures
- Excludes: `spark-itest`, `breez-sdk-itest`, WASM packages
- Runs `check-git-status` after to detect generated file changes

### wasm-test
- WASM compilation errors
- Browser compatibility (headless Firefox)
- Node.js compatibility
- Uses wasm-pack for test execution

### flutter
- Flutter binding generation errors (`flutter_rust_bridge_codegen`)
- Rust compilation for Flutter target
- Runs: `make generate-bindings-build-release`

### docs-* (per language)

| Language | Job | Validation |
|----------|-----|------------|
| rust | `docs-rust` | `cargo clippy` on snippets |
| wasm | `docs-wasm` | TypeScript compilation, ESLint |
| flutter | `docs-flutter` | `dart analyze --fatal-infos` |
| go | `docs-go` | `go build` on snippets |
| python | `docs-python` | mypy type checking, pylint |
| kotlin-mpp | `docs-kotlin-mpp` | Gradle build |
| swift | `docs-swift` | `swift build` and `swift run` |
| react-native | `docs-react-native` | TypeScript compilation, ESLint |
| csharp | `docs-csharp` | `dotnet build`, `dotnet format` |

## Make Targets

See `./CLAUDE.md` for the full command reference. This table shows underlying implementations:

| Target | Underlying Command | Purpose |
|--------|-------------------|---------|
| `make check` | fmt-check + clippy-check + test + flutter-check | Full local validation |
| `make cargo-clippy-check` | `cargo xtask clippy` | Rust clippy |
| `make wasm-clippy-check` | `cargo xtask wasm-clippy` | WASM clippy |
| `make fmt-check` | `cargo xtask fmt --check` | Format check |
| `make cargo-test` | `cargo xtask test` | Unit tests |
| `make wasm-test` | `cargo xtask wasm-test` (browser + node) | WASM tests |
| `make itest` | `cargo xtask itest` | Integration tests |
| `make breez-itest` | `cargo xtask test -p breez-sdk-itest` | Breez service tests |

## xtask Commands

Located in `crates/xtask/src/main.rs`:

| Command | Description |
|---------|-------------|
| `cargo xtask test` | Run unit tests (excludes itest packages) |
| `cargo xtask wasm-test` | Run WASM tests via wasm-pack |
| `cargo xtask clippy` | Run clippy on workspace |
| `cargo xtask wasm-clippy` | Run clippy for WASM target |
| `cargo xtask fmt` | Run rustfmt |
| `cargo xtask build` | Build workspace |
| `cargo xtask itest` | Run integration tests with Docker |
| `cargo xtask check-doc-snippets -p <lang>` | Validate doc snippets |
| `cargo xtask flutter-check` | Check Flutter package |

## What CI Does NOT Catch

Focus PR review on these areas:

| Area | Why CI Misses It |
|------|------------------|
| Design decisions | Requires human judgment |
| Unintentional behavior changes | Tests may not cover edge cases |
| Security concerns | Static analysis has limits |
| Binding semantic consistency | CI checks compilation, not logic |
| Over-engineering | Subjective assessment |
| Missing error handling | Only fails if tests cover the path |
| API usability | Requires human evaluation |

## GitHub Actions

| Action | Location | Purpose |
|--------|----------|---------|
| `setup-build` | `.github/actions/setup-build/action.yaml` | Install protoc, setup rust-cache |
| `check-git-status` | `.github/actions/check-git-status/action.yaml` | Fail if tests modified files |

## Environment Variables (breez-test)

| Variable | Purpose |
|----------|---------|
| `FAUCET_USERNAME` | Faucet auth |
| `FAUCET_PASSWORD` | Faucet auth |
| `FAUCET_CONCURRENCY` | Rate limiting (default: 2) |
| `RECOVERY_TEST_MNEMONIC` | Recovery test seed |
| `RECOVERY_TEST_EXPECTED_PAYMENTS` | Expected payment count |
