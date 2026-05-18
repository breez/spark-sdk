# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Dependencies

The following system dependencies are required:

```bash
# Debian/Ubuntu
apt-get install -y protobuf-compiler

# macOS
brew install protobuf

# Arch Linux
pacman -S protobuf
```

## Build Commands

```bash
make build              # Build workspace (excludes WASM)
make build-release      # Release build with LTO
make build-wasm         # Build for WASM target
```

## Testing

```bash
make cargo-test         # Run Rust unit tests
make wasm-test          # Run WASM tests (browser + Node.js)
make itest              # Integration tests (requires Docker)
make breez-itest        # Breez integration tests (requires faucet credentials)
```

Run a single test:
```bash
cargo test <test_name> -p <package>
```

## Code Quality

```bash
make check              # Run all checks (fmt, clippy, tests) - use before committing
make fmt-check          # Check formatting
make fmt-fix            # Fix formatting
make clippy-check       # Run clippy lints (cargo + WASM)
make clippy-fix         # Fix clippy issues
```

## Architecture

### Crate Structure

- **crates/breez-sdk/core** - Main SDK library with public API (`BreezSdk`)
- **crates/breez-sdk/common** - Shared utilities, LNURL support, networking, sync protocol
- **crates/breez-sdk/bindings** - UniFFI bindings for Go, Kotlin, Python, React Native, Swift
- **crates/breez-sdk/wasm** - WebAssembly bindings for JavaScript/TypeScript
- **crates/breez-sdk/cli** - Command-line interface for testing
- **crates/spark** - Low-level Spark protocol (addresses, signing, operators, tokens)
- **crates/spark-wallet** - High-level wallet operations wrapping Spark protocol
- **crates/xtask** - Custom build tasks (powers `make` commands via `cargo xtask`)

### Key Abstractions

- `Storage` trait - Pluggable persistence layer (see Storage Implementations below)
- `Signer` trait - Cryptographic operations (FROST threshold signing)
- `BitcoinChainService` trait - Blockchain provider interface
- `EventEmitter` - Broadcasts `SdkEvent` (Synced, PaymentSucceeded, PaymentFailed, etc.)

### Storage Implementations

The `Storage` trait (`crates/breez-sdk/core/src/persist/mod.rs`) has multiple implementations. **When adding new Storage functionality, all implementations must be updated.**

| Implementation | Location | Platform | DB |
|---|---|---|---|
| SQLite (Rust) | `crates/breez-sdk/core/src/persist/sqlite.rs` | Native (macOS, Linux, Windows) | SQLite |
| PostgreSQL (Rust) | `crates/breez-sdk/core/src/persist/postgres.rs` | Server (feature-gated: `postgres`) | PostgreSQL |
| Web (JS) | `crates/breez-sdk/wasm/js/web-storage/index.js` | Browser (WASM) | IndexedDB |
| Node SQLite (JS) | `crates/breez-sdk/wasm/js/node-storage/index.cjs` | Node.js (WASM) | SQLite (`better-sqlite3`) |
| Node Postgres (JS) | `crates/breez-sdk/wasm/js/postgres-storage/index.cjs` | Node.js (WASM) | PostgreSQL (`pg`) |

All implementations run the **same shared test suite** in `crates/breez-sdk/core/src/persist/tests.rs`. When modifying storage:

1. Update every implementation listed above
2. Add test coverage to the shared test suite (`tests.rs`)
3. Add calls to any new test functions in **each** implementation's test harness:
   - Rust SQLite: `crates/breez-sdk/core/src/persist/sqlite.rs` (test module at bottom)
   - Rust Postgres: `crates/breez-sdk/core/src/persist/postgres.rs` (test module at bottom)
   - Web: `crates/breez-sdk/wasm/src/persist/tests/web.rs`
   - Node SQLite: `crates/breez-sdk/wasm/src/persist/tests/node.rs`
   - Node Postgres: `crates/breez-sdk/wasm/src/persist/tests/postgres.rs`

JS implementations also have migration files (`migrations.cjs`) alongside their `index.cjs`.

### Data Flow

```
BreezSdk (API) → SparkWallet (wallet ops) → Spark (protocol) → Operators (gRPC)
     ↓
Storage → SyncedStorage → Breez Sync Service (multi-device)
```

## Updating SDK Interfaces

When changing the SDK's public interface, update these files:

1. **crates/breez-sdk/core/src/models.rs** - Add UniFFI macros to interface types
2. **crates/breez-sdk/wasm/src/models.rs** - Update exported structs/enums
3. **crates/breez-sdk/wasm/src/sdk.rs** - Update WASM interface
4. **packages/flutter/rust/src/models.rs** - Update mirrored structs/enums
5. **packages/flutter/rust/src/sdk.rs** - Update Flutter interface

## Documentation Inline Syntax

When writing mdbook documentation in `docs/breez-sdk/src/`, use these preprocessor macros for language-aware inline code that adapts to the selected language tab:

- `{{#name identifier}}` - For functions, methods, parameters, properties
  - Rust/Python: `get_info` (snake_case)
  - Swift/Kotlin/JS/Flutter: `getInfo` (camelCase)
  - Go/C#: `GetInfo` (PascalCase)

- `{{#enum Type::Variant}}` - For enum variants
  - Rust: `SdkEvent::Synced`
  - Python: `SdkEvent.SYNCED`
  - Swift: `SdkEvent.synced`
  - Go: `SdkEventSynced`
  - Others: `SdkEvent.Synced`

Example:
```markdown
Call {{#name get_info}} after each {{#enum SdkEvent::Synced}} event.
```

See [snippets-processor/src/main.rs](docs/breez-sdk/snippets-processor/src/main.rs) for transformation rules.

## Generated Files Policy

Files with a `Generated by <tool>` header get overwritten on the next codegen run. Never edit them in place. Check for the header with `grep -rln "Generated by" packages/` before touching anything under `packages/react-native/` or `packages/flutter/`.

Two ways to handle changes:

1. **Move the code out of the codegen tree.** If you're adding a whole hand-written file, place it in a source root the generator doesn't own. For React Native Android that means `src/main/kotlin/` instead of `src/main/java/` (the latter is wiped by `yarn ubrn:clean`). Example: `BreezSdkSparkPasskeyModule.kt` and `CredentialManagerPrfCore.kt`.

2. **Add a post-codegen patch script.** When the edit has to live inside a generated file (e.g. `build.gradle` dependencies, `TurboReactPackage.getModule()`, `src/index.tsx` exports), commit a script that re-applies the edit after every regen. Canonical example: `packages/react-native/scripts/post-ubrn.js`, wired into `ubrn:android` / `ubrn:ios` / `ubrn:checkout` in `package.json`. Every patch must be idempotent (runs as a no-op on the second pass) and anchor-guarded (fails loudly if the anchor text is missing, so upstream format changes surface as build errors instead of silent regressions).

The committed tree always has every edit applied, so fresh clones build without running the script. The script is a safety net for future regens.

## CLI Modification Policy

**Do not modify language-specific CLIs** (`crates/breez-sdk/bindings/examples/cli/langs/`) unless:
- Fixing failures in the **CLI matrix** or **Flutter** (which includes Flutter/Dart CLI static analysis) CI jobs. Always fix those CI failures, as this gives the Sync CLI Languages workflow (`sync-cli.yml`) better context when propagating future Rust CLI changes. Keep fixes minimal: only make changes needed to pass the build. Do not add new features, flags, or update descriptions; leave full feature propagation to the sync workflow.
- Explicitly requested by the user (e.g. porting a new feature for testing).

The **Sync CLI Languages** workflow (`sync-cli.yml`) automatically propagates Rust CLI changes to all language CLIs. Unnecessary modifications to language CLIs create PR noise.

The **Rust CLI** (`crates/breez-sdk/cli/`) can be modified freely as it is the source of truth that the sync workflow reads from.

## Workspace Configuration

- Rust edition 2024, MSRV 1.88
- Clippy: pedantic + suspicious + complexity + perf warnings enabled
- Release builds use LTO and `opt-level = "z"` for size optimization
- Uses `cargo xtask` for build automation (aliased in `.cargo/config.toml`)

## Comment Style

Comments are for the reader, not the writer. Apply these rules to any new or modified comment, including doc-comments and module headers.

### 0. No em-dashes or en-dashes

Do not use em-dashes (`—`) or en-dashes (`–`) anywhere in the codebase: code, comments, doc-comments, commit messages, PR descriptions, guide markdown, snippets. Use the punctuation that fits the relationship between clauses:

- Explanatory aside: colon. `X is not supported: Y is the reason.`
- Two independent clauses: period. `Do A. Do B.`
- Contrast: comma + conjunction. `A is true, but B is also true.`
- Parenthetical: parentheses. `A (which is the reason) ...`
- Numeric range: "to". `3 to 5 lines`, not `3–5 lines`.

This applies before every rule that follows.

### 1. Cut what the code already says

Default to writing no comment. Add one only when the WHY is non-obvious: a hidden constraint, a workaround for a specific bug, a subtle invariant, behavior that would surprise a reader. If removing the comment wouldn't confuse a future reader, delete it. A long comment block is a code smell: it usually means the underlying code wants to be split into smaller, better-named units instead.

**Audit attached docs when signatures change.** When you remove or rename a parameter, struct field, or enum variant, also grep nearby `@param`, `///`, `/**`, JSDoc, KDoc blocks for the old name. A stale `@param autoRegister` describing a parameter that no longer exists is worse than no doc: it lies about the current signature. Same for `excludeCredentialIds` that became per-call instead of per-instance: the constructor doc has to follow.

### 2. Compact what stays

Genuinely-necessary detail: target 3 to 5 lines. If you can't, the comment is doing the job of a separate doc: link out instead of inlining a whole essay. No multi-paragraph docstrings. One short sentence beats a tight paragraph beats a sprawling block.

### 3. Calibrate to the reader

Different surfaces have different audiences. Match the language:

| Surface | Audience | Pitch |
|---|---|---|
| Public API doc-comments (`///`, `/**`, Dart `///`, Swift `///`) | App developers integrating the SDK | What it does + why the caller cares. Skip platform-internal jargon. |
| Internal `//` comments | Engineers maintaining this file | Why the code is shaped this way (constraints, gotchas, references). Spec-level detail is fine. |
| Guide markdown (`docs/breez-sdk/src/guide/`) | Integrators reading the website | Conceptual, task-oriented. No implementation details unless they leak into the public API. |
| Snippets (`docs/breez-sdk/snippets/`) | Copy-paste starters | Minimal inline explanation; let the code speak. Long context belongs in the guide. |
| Commit messages / PR descriptions | Reviewers + future code-archaeologists | What changed and why. Not what the code looks like. |

When a comment is technically accurate but reads like a kernel-debug log to anyone outside the SDK team, rewrite it for the surface's audience. Pattern: "Simpler, non-tech version of: <original>" → answer *what* this does and *why* it's necessary, not *how* it works at the byte level.

### 4. Don't leak internal-looking specifics

Real production identifiers (Apple Team IDs, internal infra hostnames, employee names, customer IDs) don't belong in example comments. Use a placeholder (`<TEAM_ID>`, `your-app.com`, `<your api key>`). Same for stack-trace excerpts, error strings that contain user data, debugging breadcrumbs from one-off investigations: strip before committing.

**Don't leak language-specific type sugar in cross-language API docs either.** Public API docs cross language boundaries: UniFFI generates Swift / Kotlin, wasm-bindgen generates TypeScript, FRB generates Dart. A Rust doc that enumerates `Some(true)` / `Some(false)` / `None` on an `Option<bool>` field reads as gibberish to the Swift caller who sees `Bool?`, or the TS caller who sees `boolean | undefined`. Use plain values plus "unset" / "absent" / "missing": *"`true` restricts X. `false` allows Y. Unset uses the provider default."* Same for `Result<T, E>::Err(...)`, `Vec<u8>`, etc. The doc is read on every target, not just Rust.

### 5. Strip narrative; keep implementation facts

Comments describe the code that exists, not the history of how it got there. Implementation-focused only. Strip:

- **Development history**: "we used to do X, now we do Y because…", "originally returned Z but switched after…"
- **Step-by-step decision narrative**: "first we tried A, then B, finally C"
- **Credit-the-PR comments**: "added for #1234", "per design review", "recently fixed in PR #5678"
- **TODOs about the past**: "this used to be wrong, now corrected"
- **Chronicling intermediate choices**: alternatives considered, why they were rejected

The *only* acceptable narrative is a concise sketch (1 to 3 sentences) of a non-obvious **problem**, **why it couldn't be solved directly**, and the **workaround applied**. Frame this as a present-tense fact about the code, not a story:

> ✘ "We tried using `foo()` here but it deadlocks when called from the main thread, so we switched to `bar()`."
>
> ✓ "Uses `bar()` instead of `foo()`: `foo()` deadlocks on the main thread."

#### Pointers to active external context are fine

The "no PR/ticket context" rule is about crediting the PR that *added* the code, not about linking to the source of truth a workaround depends on. Link out when the comment would otherwise have to repeat detail that lives somewhere else:

- **An open upstream bug your workaround depends on**: `// Workaround for tokio-rs/tokio#1234 (open).`
- **A spec / RFC the implementation is reading**: `// CBOR major-type-2 byte string; see RFC 8949 §3.1.`
- **A design doc or PR description with the long-form analysis**: `// Rationale: github.com/our-org/our-repo/pull/5678.`

Rule of thumb: if the link disappeared, would a future reader lose information they need to maintain the code? If yes, keep it. If it's just a chronicle of who wrote it, drop it.

Durable reasoning (a constraint, invariant, or contract) belongs in the comment as a fact. Decision *history* belongs in the commit message.

### 6. Frame what something IS, not what happens or what isn't

A doc-comment answers "what is this thing and what is it for", not "what happens downstream when you use it" and not "what other things live elsewhere."

- **Lead with meaning, not consequence.** *"Restrict the assertion to credentials already on this device"* beats *"Suppresses the cross-device picker"*: the first describes what the field IS, the second only describes what HAPPENS when you set it. Consequences are useful, but they come after the meaning, not before it.
- **Don't document absence.** Sentences like *"Provider-scoped knobs (`rp_id`, `credentialRegistry`) live on the platform constructor"* on a struct that doesn't carry those knobs describe what *isn't* here. They're usually leftover artifacts of a refactor that moved fields out: they pollute the doc of the thing that *is* present, and the reader who needs `rp_id` will search for it anyway.
- **Don't restate the type.** `pub label: Option<String>` doesn't need *"An optional string label"* on top; the signature already says that. Document why the field is optional, what `None` means at the domain level, what the default is.

---

## SDK Usage Guide (For Integrators)

This section is for developers integrating the Breez SDK into their apps.

### API Key (Required)

A Breez API key is required for the SDK to work. Request one for free at:
**https://breez.technology/request-api-key/#contact-us-form-sdk**

### Installation

| Platform | Package |
|----------|---------|
| JavaScript/WASM | `npm install @breeztech/breez-sdk-spark` |
| React Native | `npm install @breeztech/breez-sdk-spark-react-native` |
| Python | `pip install breez-sdk-spark` |
| Go | `go get github.com/breez/breez-sdk-spark-go` |
| C# | `dotnet add package Breez.Sdk.Spark` |
| Swift | SPM: `https://github.com/breez/breez-sdk-spark-swift.git` |
| Kotlin | Maven: `https://mvn.breez.technology/releases` |
| Flutter | Git: `https://github.com/breez/breez-sdk-spark-flutter` |
| Rust | Git: `https://github.com/breez/spark-sdk` |

### Quick Start

See working examples in `docs/breez-sdk/snippets/` - these are compiled/tested and always up to date:

| Task | TypeScript | Rust |
|------|------------|------|
| Initialize | `wasm/getting_started.ts` | `rust/src/getting_started.rs` |
| Send payment | `wasm/send_payment.ts` | `rust/src/send_payment.rs` |
| Receive payment | `wasm/receive_payment.ts` | `rust/src/receive_payment.rs` |
| List payments | `wasm/list_payments.ts` | `rust/src/list_payments.rs` |
| Parse input | `wasm/parsing_inputs.ts` | `rust/src/parsing_inputs.rs` |
| LNURL-Pay | `wasm/lnurl_pay.ts` | `rust/src/lnurl_pay.rs` |
| Buy Bitcoin | `wasm/buying_bitcoin.ts` | `rust/src/buying_bitcoin.rs` |
| Events | `wasm/getting_started.ts` (search `addEventListener`) | `rust/src/getting_started.rs` (search `EventListener`) |

**Minimal TypeScript example:**

```typescript
import { connect, defaultConfig } from '@breeztech/breez-sdk-spark'

const config = defaultConfig('mainnet')
config.apiKey = '<your api key>'

const sdk = await connect({
  config,
  seed: { type: 'mnemonic', mnemonic: '<12/24 words>', passphrase: undefined },
  storageDir: './.data'
})

const info = await sdk.getInfo({ ensureSynced: true })
// info.balanceSats, info.tokenBalances

// To get addresses:
// const lnAddress = await sdk.getLightningAddress()
// const sparkAddr = await sdk.receivePayment({ paymentMethod: { type: 'sparkAddress' } })

await sdk.disconnect()
```

**Minimal Rust example:**

```rust
use breez_sdk_spark::*;

let mut config = default_config(Network::Mainnet);
config.api_key = Some("<your api key>".to_string());

let sdk = connect(ConnectRequest {
    config,
    seed: Seed::Mnemonic { mnemonic: "<words>".into(), passphrase: None },
    storage_dir: "./.data".to_string(),
}).await?;

let info = sdk.get_info(GetInfoRequest { ensure_synced: Some(true) }).await?;
// info.balance_sats, info.token_balances

sdk.disconnect().await?;
```

### Core API Methods

| Method | Description |
|--------|-------------|
| `connect(config, seed, storageDir)` | Initialize SDK |
| `disconnect()` | Clean shutdown |
| `getInfo()` | Get balance (sats) and token balances |
| `getLightningAddress()` | Get registered lightning address |
| `receivePayment(paymentMethod)` | Generate invoice, BTC address, or Spark address |
| `sendPayment(prepareResponse)` | Send payment (call prepareSendPayment first) |
| `prepareSendPayment(destination)` | Prepare a payment, get fees |
| `parse(input)` | Parse any input (invoice, address, LNURL) |
| `listPayments(filter)` | Get transaction history |
| `addEventListener(listener)` | Subscribe to events |
| `buyBitcoin(request)` | Get MoonPay URL to buy Bitcoin |

### SDK Events

- `synced` - Data synchronized, refresh UI
- `paymentSucceeded` - Payment completed
- `paymentFailed` - Payment failed
- `paymentPending` - Payment awaiting confirmation
- `claimedDeposits` - On-chain deposits claimed
- `unclaimedDeposits` - Deposits need manual claim

### Code Examples

Working code examples for all platforms are in `docs/breez-sdk/snippets/`:
- `rust/src/` - Rust examples
- `wasm/` - TypeScript/JavaScript examples
- `swift/` - Swift examples
- `kotlin_mpp_lib/` - Kotlin examples
- `flutter/lib/` - Dart examples
- `python/src/` - Python examples
- `go/` - Go examples
- `csharp/` - C# examples
- `react-native/` - React Native examples

### Common Gotchas

1. **WASM Web requires `init()`** - Call `await init()` before any SDK methods in browser environments (not needed for Node.js/Deno)

2. **Node.js version** - WASM and React Native require Node.js >= 22

3. **Storage paths** - On mobile (Android/iOS), use app-specific sandbox directories, not arbitrary paths

4. **One SDK instance per storage** - Each SDK instance needs its own unique `storageDir`

5. **Prepare before send** - Always call `prepareSendPayment()` first to get fees, then `sendPayment()` with the response

6. **Balance after sync** - Call `getInfo({ ensureSynced: true })` to get accurate balance, or listen for `synced` events

7. **Lightning address registration** - Call `registerLightningAddress()` to get a Lightning address; it's not automatic

### Networks

| Network | Config | Use Case |
|---------|--------|----------|
| `mainnet` | `defaultConfig('mainnet')` | Production |
| `testnet` | `defaultConfig('testnet')` | Testing with testnet Bitcoin |
| `regtest` | `defaultConfig('regtest')` | Development (no API key needed, use [Lightspark faucet](https://app.lightspark.com/regtest-faucet)) |

**Regtest** is recommended for development - free to use, no real value, supports Spark payments, deposits, withdrawals, and token issuance.

**Mainnet with small amounts** is recommended for Lightning testing (regtest has limited Lightning network).

### Error Handling

The SDK throws `SdkError` with these variants:

| Error | Meaning |
|-------|---------|
| `InsufficientFunds` | Not enough balance for payment |
| `InvalidInput` | Bad parameter (address, amount, etc.) |
| `NetworkError` | Connection/API issues |
| `StorageError` | Database/persistence issues |
| `MaxDepositClaimFeeExceeded` | On-chain fees too high for auto-claim |

```typescript
try {
  await sdk.sendPayment({ prepareResponse })
} catch (error) {
  if (error.message.includes('InsufficientFunds')) {
    // Handle insufficient balance
  }
}
```

### LNURL Operations

| Operation | Flow |
|-----------|------|
| **LNURL-Pay** | `parse(url)` → check `type === 'lnurlPay'` or `'lightningAddress'` → `prepareLnurlPay()` → `lnurlPay()` |
| **LNURL-Withdraw** | `parse(url)` → check `type === 'lnurlWithdraw'` → `lnurlWithdraw({ amountSats, withdrawRequest })` |
| **LNURL-Auth** | `parse(url)` → check `type === 'lnurlAuth'` → show domain to user → `lnurlAuth(requestData)` |

### Token Operations

**Receiving tokens:**
```typescript
// Get token balances
const info = await sdk.getInfo({ ensureSynced: true })
for (const [tokenId, balance] of Object.entries(info.tokenBalances)) {
  console.log(`${balance.tokenMetadata.ticker}: ${balance.balance}`)
}

// Receive via Spark invoice
const response = await sdk.receivePayment({
  paymentMethod: { type: 'sparkInvoice', tokenIdentifier: '<token id>', amount: '1000' }
})
```

**Sending tokens:**
```typescript
const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: '<spark address or invoice>',
  tokenIdentifier: '<token id>',
  amount: BigInt(1000)
})
await sdk.sendPayment({ prepareResponse })
```

**Issuing tokens (for token issuers):**
```typescript
const issuer = sdk.getTokenIssuer()
const metadata = await issuer.createIssuerToken({
  name: 'My Token', ticker: 'MTK', decimals: 6, isFreezable: false, maxSupply: BigInt(1_000_000)
})
await issuer.mintIssuerToken({ amount: BigInt(1000) })  // Mint to self
await issuer.burnIssuerToken({ amount: BigInt(500) })   // Burn from self
```

### Full Documentation

See `docs/breez-sdk/src/guide/` for complete documentation markdown files.