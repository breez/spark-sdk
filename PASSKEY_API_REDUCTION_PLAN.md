# Passkey API Surface Reduction — Execution Plan

Self-contained plan for a **local** Claude Code session. Everything needed to
execute is here; no prior session context required.

---

## 0. Context & base

- **Source of truth:** `origin/proto/passkey-shared-core` (`c24ed7f`).
  API-identical to `proto/passkey-shared-core-pre-rebase` — only a 1-line
  `cargo fmt` diff in `crates/breez-sdk/cli/src/passkey/fido2_prf.rs`.
- **Start locally from the source-of-truth branch** so the diff is clean:
  ```bash
  git fetch origin proto/passkey-shared-core
  git checkout -b passkey/api-reduction origin/proto/passkey-shared-core
  ```
- **Goal:** shrink user-facing `PasskeyClient` to the minimum happy-path
  surface; push low-level ops behind sub-objects or the internal `Passkey`
  orchestrator.

---

## 1. Locked design decisions

| # | Decision |
|---|----------|
| Q1 | One method: `check_availability() -> PasskeyAvailability`. Removes public `is_available` / `is_supported` / `check_domain_association` passthroughs. |
| Q2 | Drop the public `Identity` type. Keep `LabelStore` trait pluggable with a `pubkey: &[u8; 33]` boundary. |
| Q3 | Known-cred get/remove/clear via `client.credentials()` sub-object — off the top-level surface. |
| Q4 | Label list/store via `client.labels()` sub-object — reachable from all bindings. |
| Naming | Rename `LabelStore::ensure_label_published` → `store_label`; idempotency documented, not in the name. |
| Default label | Configurable via a new **`PasskeyConfig`** struct passed to `PasskeyClient::new` (see §1.1). Falls back to the internal `DEFAULT_LABEL` const (`"Default"`) when unset. |
| Relay config | **Delete `NostrRelayConfig`** (one-field wrapper after `timeout_secs` was already dropped). Fold its `breez_api_key` into `PasskeyConfig`. Removes a public type from all 5 bindings. |
| Provider naming | The platform provider class is already **`PasskeyProvider`** on all 5 platforms (iOS/Android/JS/Flutter/RN) — verify, do **not** rename. Residual `prf` cruft (filenames, `PasskeyPrfException`, JS subpath) handled in opt-in Stage D (§6). |

### Confirmed decisions

- **R1 — Nostr signing — CONFIRMED: Option A.** The default Nostr label
  store stays **concrete & internal** — the orchestrator owns the
  account-master-derived `nostr::Keys` and builds `NostrSaltClient`
  directly; the secret never crosses a trait boundary. The pluggable
  `LabelStore` trait is **Rust-only** (`with_label_store`), for
  server-mediated stores, and its boundary carries only `pubkey: &[u8; 33]`
  as a stable user id. (Rejected: B = secret across trait; C = renamed
  opaque Identity newtype.)
- **R2 — Delivery — CONFIRMED: 3 staged commits.** Stage A core + WASM,
  Stage B Flutter / RN / native, Stage C CLI + mirrors + docs. Each stage
  must compile independently.

### 1.1 `PasskeyConfig`

New public record (added to every binding) — **replaces** the bare
`relay_config: Option<NostrRelayConfig>` constructor parameter and the
now-deleted `NostrRelayConfig` type. Exactly two optional fields; nothing
else belongs here (all other knobs — `rp_id`, `auto_register`,
`credential_registry`, etc. — are provider-scoped, see §1.3):

```rust
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasskeyConfig {
    /// Breez API key for the authenticated Breez Nostr relay (NIP-42).
    /// `None` ⇒ public relays only (label sync still works, less robust).
    /// Only consulted by the default Nostr label store; ignored when a
    /// custom `LabelStore` is injected via `with_label_store`.
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub breez_api_key: Option<String>,
    /// Wallet label used when register/sign_in receive `label = None`.
    /// `None` ⇒ internal `DEFAULT_LABEL` ("Default").
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub default_label: Option<String>,
}
```

The resolved default label is stored on `Passkey` at construction and used
everywhere `DEFAULT_LABEL` is currently referenced (`setup_wallet`, the
`sign_in` discovery path). `from_config(&crate::Config)` builds a
`PasskeyConfig` (`config.api_key` → `breez_api_key`, `default_label =
None`).

### 1.2 Canonical naming (aligned across all platforms)

| Concept | Canonical name | Status |
|---|---|---|
| Orchestrator entry point | `PasskeyClient` | exists |
| Platform provider class | `PasskeyProvider` | **already consistent on all 5 platforms** — verify, do NOT rename |
| PRF contract trait/interface | `PrfProvider` | exists |
| Client config record | `PasskeyConfig` | NEW |
| Availability result | `PasskeyAvailability` | NEW |
| Label sub-object | `PasskeyLabels` | NEW |
| Credential sub-object | `PasskeyCredentials` | NEW |
| Label-store trait (Rust-only) | `LabelStore` | exists, reshaped |
| Provider exception (Flutter/RN) | `PasskeyException` | Stage D rename (from `PasskeyPrfException`) |
| JS provider subpath | `@breeztech/breez-sdk-spark/passkey-provider` | Stage D rename (from `passkey-prf-provider`) |
| Removed type | ~~`NostrRelayConfig`~~, ~~`Identity`~~ | deleted |

### 1.3 Provider-scoped knobs — explicitly NOT moved to `PasskeyConfig`

These stay on the platform `PasskeyProvider` constructor (provider concern;
`PasskeyClient` is provider-agnostic — CLI File/YubiKey/FIDO2 providers have
no `rp_id`). Do **not** lift them up: `rp_id`, `rp_name`, `user_name`,
`user_display_name`, `auto_register`, `allow_credential_ids`,
`credential_registry`, `on_registry_error`, `hints`, `default_timeout_ms`,
`authenticator_attachment`, `team_id`.

---

## 2. Target public `PasskeyClient` surface

All bindings (UniFFI / WASM / Flutter / RN):

```
PasskeyClient
  new(prf_provider, config?)        // config: Option<PasskeyConfig> (see §1.1)
  check_availability()  -> PasskeyAvailability
  register(RegisterRequest)  -> RegisterResponse
  sign_in(SignInRequest)     -> SignInResponse
  labels()      -> PasskeyLabels
  credentials() -> PasskeyCredentials
```

Rust-only (separate non-`uniffi::export` impl block): `from_config`,
`with_label_store`, `passkey()` escape hatch.

**Removed from public surface:** `list_labels`, `store_label`,
`is_available`.

```
enum PasskeyAvailability {
  Available,
  PrfUnsupported,
  NotAssociated { source: String, reason: String },
  Skipped       { reason: String },
}

PasskeyLabels                       PasskeyCredentials
  list()  -> Vec<String>              get()    -> Vec<Vec<u8>>
  store(label)                        remove(credential_id)
                                      clear()
```

`check_availability()` = `PrfProvider::is_supported()` → `false` ⇒
`PrfUnsupported`; else map `PrfProvider::check_domain_association()`
(`Associated`→`Available`, pass through `NotAssociated` / `Skipped`).

---

## 3. Stage A — Core + WASM (commit 1)

### Core — `crates/breez-sdk/core/src/passkey/`

- **`passkey_prf_provider.rs`** — keep trait as-is; add three default-no-op
  methods so file/YubiKey/FIDO2 providers inherit nothing:
  ```rust
  async fn get_known_credential_ids(&self) -> Result<Vec<Vec<u8>>, PrfProviderError> { Ok(vec![]) }
  async fn remove_known_credential_id(&self, _id: Vec<u8>) -> Result<(), PrfProviderError> { Ok(()) }
  async fn clear_known_credential_ids(&self) -> Result<(), PrfProviderError> { Ok(()) }
  ```
- **`label_store.rs`** — delete `Identity` struct + impl. Trait becomes:
  ```rust
  trait LabelStore: Send + Sync {
      async fn list_labels(&self, pubkey: &[u8; 33]) -> Result<Vec<String>, PasskeyError>;
      /// Idempotent: no-op if `label` already published for `pubkey`.
      async fn store_label(&self, pubkey: &[u8; 33], label: &str) -> Result<(), PasskeyError>;
  }
  ```
- **`nostr_client.rs`** — `NostrSaltClient` keeps the **full `nostr::Keys`**
  (owned, passed in by the orchestrator). Do **not** implement the new
  `LabelStore` trait for it (can't sign with a pubkey-only boundary).
  Provide concrete `list_labels()` / `store_label(label)` using owned keys.
- **`mod.rs` (`Passkey`)** — drop public `Identity` re-export. Replace
  `derive_identity()` with `derive_keys()` returning cached
  `OnceCell<nostr::Keys>`. Introduce a backend split:
  ```rust
  enum LabelBackend { Nostr(NostrSaltClient), Custom(Arc<dyn LabelStore>) }
  ```
  Default path → `Nostr` (owned keys). Custom path → derive 33-byte pubkey
  from cached keys, pass to trait. Keep `setup_wallet`, `list_labels`,
  `store_label`, `is_available` on `Passkey` (internal, **not** in
  bindings). Add `check_availability()` and known-cred passthroughs
  (delegate to `prf_provider`). Store the resolved default label
  (`config.default_label.unwrap_or(DEFAULT_LABEL)`) on `Passkey`; replace
  every existing `DEFAULT_LABEL` use (`setup_wallet`, `sign_in` discovery)
  with the stored value. `Passkey::new` / `with_label_store` /
  `from_config` take `Option<PasskeyConfig>`.
- **`models.rs`** — add `PasskeyAvailability` (`#[cfg_attr(feature="uniffi",
  derive(uniffi::Enum))]`), reuse the `source`/`reason` shape from
  `DomainAssociation`. Add **`PasskeyConfig`** (`uniffi::Record`, see §1.1).
  **Delete `NostrRelayConfig`** entirely; remove its `pub use` re-export
  and every reference (the only field, `breez_api_key`, now lives on
  `PasskeyConfig`). `NostrSaltClient` takes `breez_api_key: Option<String>`
  directly.
- **`passkey_client.rs`**:
  - `#[uniffi::export]` impl block: `new(prf_provider, config?)` where
    `config: Option<PasskeyConfig>`, `check_availability`, `register`,
    `sign_in`, `labels()`, `credentials()`. **Remove** `list_labels`,
    `store_label`, `is_available`.
  - Plain `impl` block (Rust-only): `from_config` (builds `PasskeyConfig`
    from `crate::Config`), `with_label_store`, `passkey()`.
  - New `#[uniffi::Object]` types `PasskeyLabels` (holds a `Passkey`
    clone → `list`, `store`) and `PasskeyCredentials` (holds
    `Arc<dyn PrfProvider>` → `get`, `remove`, `clear`).
  - Update the in-file test mocks for the new trait shape.

### WASM — `crates/breez-sdk/wasm/`

- **`src/passkey.rs`** — remove `listLabels` / `storeLabel` / `isAvailable`
  js methods. Constructor takes `PasskeyConfig` in place of
  `NostrRelayConfig`. Add `checkAvailability` (returns
  `PasskeyAvailability`), `labels()` / `credentials()` returning
  `#[wasm_bindgen]` sub-structs with their methods. Add
  `PasskeyAvailability` and `PasskeyConfig` extern bindings; **delete the
  `NostrRelayConfig` extern binding**.
- **`src/models/passkey_prf_provider.rs`** — add Reflect-based optional
  probes + bridging for `getKnownCredentialIds` /
  `removeKnownCredentialId` / `clearKnownCredentialIds` (mirror the
  existing `createPasskey` optional-probe pattern).
- **`js/passkey-prf-provider/index.d.ts` + `index.js`** — remove
  `deriveSeed(salt)` from the class and its doc refs; keep
  `deriveSeeds(salts)`. Add `getKnownCredentialIds` /
  `removeKnownCredentialId` / `clearKnownCredentialIds` to the class,
  delegating to the configured `credentialRegistry`.
- **`js/passkey-capacitor-bridge/index.d.ts`** — remove `deriveSeed`; keep
  `deriveSeeds`. Keep the three known-cred methods (they now back
  `client.credentials()`).

**Stage A verification:**
```bash
make fmt-fix && make clippy-check
cargo test -p breez-sdk-spark passkey
make build-wasm
```

---

## 4. Stage B — Flutter / RN / native providers (commit 2)

- **`packages/flutter/rust/src/passkey.rs`** — drop `list_labels` /
  `store_label` / `is_available`; add `check_availability`, `labels()`,
  `credentials()`. Constructor takes `PasskeyConfig`. Extend
  `CallbackPrfProvider` with optional known-cred callbacks.
- **`packages/flutter/rust/src/models.rs`** + **`src/sdk.rs`** — mirror
  `PasskeyAvailability`, **`PasskeyConfig`**, and the sub-object handles.
- **`packages/flutter/lib/src/passkey_prf_provider.dart`** — drop
  single-salt; add known-cred passthrough.
- **`packages/flutter/{android,ios}/.../BreezSdkSparkPasskeyPlugin.{kt,swift}`**
  — wire known-cred methods to the native `KnownCredentialsStore`.
- **`packages/react-native/src/passkey-prf-provider.ts`** — drop
  `deriveSeed`; add known-cred passthrough.
- **`packages/react-native/.../BreezSdkSparkPasskeyModule.kt` /
  `ios/BreezSdkSparkPasskey.swift`** — known-cred wiring.
- **`crates/breez-sdk/bindings/langs/swift/.../PasskeyProvider.swift`** and
  **`bindings/langs/shared/android-passkey/.../PasskeyProvider.kt`** — add
  `get/remove/clearKnownCredentialIds` delegating to the existing
  `KnownCredentialsStore`. Confirm no public single-salt (`deriveSeeds`
  only).

**Stage B verification:** `make build` ; Flutter `dart analyze` (if local
toolchain available); RN/TS typecheck.

---

## 5. Stage C — CLI + interface mirrors + docs (commit 3)

- **`crates/breez-sdk/cli/src/passkey/mod.rs`** — replace
  `client.list_labels()` / discovery wiring with `client.labels().list()`;
  replace any `is_available` usage with `check_availability()`; update the
  `PasskeyClient::new` call site to pass `PasskeyConfig` (the CLI's
  `--label` plumbing can populate `default_label`). (Rust CLI is the
  source of truth — modification allowed.)
- **Interface checklist (CLAUDE.md "Updating SDK Interfaces"):** confirm
  `PasskeyAvailability`, `PasskeyConfig` + sub-objects exported from
  `crates/breez-sdk/core/src/passkey/models.rs`,
  `crates/breez-sdk/wasm/src/models.rs`, `wasm/src/sdk.rs`,
  `packages/flutter/rust/src/models.rs`, `packages/flutter/rust/src/sdk.rs`.
- **Docs/snippets:** run the **`update-snippets`** skill to regenerate the
  9 languages, then hand-adjust `docs/breez-sdk/snippets/*/passkey.*` —
  replace `listLabels`/`storeLabel` with `labels().list()`/`labels().store()`,
  `isAvailable` → `checkAvailability`, and update the `PasskeyClient`
  constructor call to pass `PasskeyConfig` (incl. a `default_label`
  example). Update
  `docs/breez-sdk/src/guide/passkey.md` and `uxguide_passkey.md` prose.
- **Language CLI examples** (`bindings/examples/cli/langs/*`) — touch ONLY
  if the CLI-matrix / Flutter CI job fails; keep minimal. Full propagation
  is handled by `sync-cli.yml`.

**Stage C verification:** `make check` (fmt + clippy + tests). Snippet
build per `docs/breez-sdk`.

---

## 6. Stage D — naming alignment (commit 4, opt-in)

Cosmetic-only; touches two public names + file moves. Land it as a
separate, clearly-scoped commit (or skip without affecting Stages A–C).

- **Verify, do NOT rename:** provider class is already `PasskeyProvider`
  on iOS (`PasskeyProvider.swift`), Android (`PasskeyProvider.kt`),
  Browser JS, Flutter (`passkey_prf_provider.dart` → `class
  PasskeyProvider`), RN (`passkey-prf-provider.ts` → `class
  PasskeyProvider`). Just confirm consistency.
- **Public renames:**
  - `PasskeyPrfException` → `PasskeyException` (Flutter
    `lib/src/passkey_prf_provider.dart`, RN `src/passkey-prf-provider.ts`).
  - JS subpath `@breeztech/breez-sdk-spark/passkey-prf-provider` →
    `…/passkey-provider` (update `packages/wasm/package.json` exports +
    all docs/snippets import lines).
- **Internal file moves (no API change):**
  `crates/breez-sdk/core/src/passkey/passkey_prf_provider.rs` →
  `prf_provider.rs`; `packages/flutter/lib/src/passkey_prf_provider.dart`
  → `passkey_provider.dart`; `packages/react-native/src/passkey-prf-provider.ts`
  → `passkey-provider.ts`; the WASM `js/passkey-prf-provider/` dir →
  `js/passkey-provider/`. Update all `mod`/`import`/`export` paths.
- Update docs/snippets (9 langs) + `guide/passkey.md` import lines.

**Stage D verification:** `make check`; `make build-wasm`; snippet build;
grep for residual `passkey_prf_provider` / `PasskeyPrfException` /
`passkey-prf-provider`.

---

## 7. Decisions — all confirmed

All design questions are resolved; no open items. Execute the plan
top-to-bottom.

- Q1–Q4: locked (see §1 table).
- R1 = **Option A** (concrete-internal Nostr default + Rust-only
  pubkey-boundary `LabelStore` trait).
- R2 = **3 staged commits** (Stages A / B / C) **+ opt-in Stage D**
  (naming alignment, commit 4). Each stage independently compilable.
- Default label + relay key = **`PasskeyConfig { breez_api_key?,
  default_label? }`** passed to `PasskeyClient::new`. **`NostrRelayConfig`
  deleted** (folded in). Nothing else moves to `PasskeyConfig` — other
  knobs are provider-scoped (§1.3).
- Provider class name `PasskeyProvider` is **already aligned** across all
  5 platforms; only residual `prf` cruft (Stage D) remains.

---

## 8. Git workflow

- One commit per stage; clear messages.
- Push: `git push -u origin passkey/api-reduction` (retry 2s/4s/8s/16s on
  network error).
- Do **not** open a PR unless explicitly requested.
