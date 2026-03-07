## Task: Sync {{LANG_NAME}} CLI with Rust CLI changes

Your job is to update the {{LANG_NAME}} CLI to match the current Rust CLI.

### What changed
${{ steps.diff-info.outputs.diff_summary }}

### Step 1: Learn the {{LANG_NAME}} SDK API
Before comparing anything, read these {{LANG_NAME}} SDK snippets at `{{SNIPPET_DIR}}`:
- `sdk_building` — builder pattern, storage setup, configuration
- `getting_started` — initialization, connection, event listening
- `passkey` — passkey PRF provider interface, seed derivation pattern

These snippets are compiled and tested — they are the ground truth for what methods exist and how they are called in {{LANG_NAME}}. You will need this context to identify real divergences vs naming conventions.

### Step 2: Analyze the changes
If a diff base was provided, run: `git diff ${{ steps.diff-info.outputs.diff_base }} HEAD -- 'crates/breez-sdk/cli/src/' 'crates/breez-sdk/cli/README.md'`
The diff is a hint for what changed recently, but it may not reveal all differences.

**Always read the current Rust CLI source files and compare them against the {{LANG_NAME}} CLI.** The Rust CLI is the source of truth. Read each mapped file pair (see Step 3) and perform this comparison for each pair:

#### 2a. Extract SDK calls
For each file pair, list every SDK/builder method call in both versions:
- Rust: every call on `sdk`, `sdk_builder`, `config`, or any SDK type
- {{LANG_NAME}}: every corresponding call on `sdk`, `builder`, `config`, or any SDK type

#### 2b. Compare call-by-call
For each SDK call in the Rust file, find the corresponding call in the {{LANG_NAME}} file. Flag any of these as a **divergence**:
- Different function name (e.g., `with_postgres_storage()` vs `create_postgres_storage()`)
- Different arguments or parameters
- Extra steps in one version that don't exist in the other (e.g., two-step init vs one-step)
- Missing calls — an SDK call in Rust with no equivalent in {{LANG_NAME}}

#### 2c. Compare CLI flags and commands
- List every CLI flag/option in both versions and flag any missing or mismatched ones
- List every command/subcommand and flag any missing ones

#### 2d. Verify divergences against snippets
For every divergence found, check the snippets you read in Step 1 (and any additional snippets from `{{SNIPPET_DIR}}` that cover the relevant command). The snippets are always up-to-date — if the Rust CLI uses a function, assume it exists in the {{LANG_NAME}} SDK unless the snippets prove otherwise.

**Do NOT assume SDK API differences are "binding-level" or "expected."** If the Rust CLI calls `with_postgres_storage()` and the {{LANG_NAME}} CLI calls `create_postgres_storage() + with_storage()`, that is a divergence — check the snippets and fix it.

Only after completing 2a–2d should you decide which divergences to fix.

#### Before marking anything "unsupported"
When a Rust feature uses a platform-specific crate (e.g., `ctap-hid-fido2`, `yubico-manager`), do NOT immediately skip it. These CLIs serve as **reference implementations for integrators** — the pattern matters more than the specific library.

1. **Read the Rust implementation** to understand: what protocol does the crate implement? What are the inputs/outputs? What's the architecture (trait, providers, orchestration)?
2. **Check the SDK bindings**: Read the {{LANG_NAME}} SDK snippets and grep for relevant types (e.g., `PasskeyPrfProvider`, `Passkey`). If the SDK exposes a trait/protocol/interface, the CLI should implement it.
3. **Separate portable from platform-specific**: Some providers are pure crypto (e.g., file-based HMAC-SHA256) and trivially portable. Others need hardware access (USB HID, NFC). Implement the portable providers fully. For hardware-dependent providers, implement the provider skeleton with {{UNSUPPORTED_HANDLER}} as the transport layer, but still implement the correct interface so integrators can see the pattern.
4. **Always implement the orchestration**: If Rust has a `resolve_passkey_seed()` that handles wallet discovery, selection, and seed derivation via SDK types — implement the equivalent in {{LANG_NAME}}. The orchestration uses SDK APIs, not platform crates.
5. **Research {{LANG_NAME}} packages**: Check if well-known packages exist on {{PACKAGE_REGISTRY}} for the protocol in question. The Rust crate's README or Cargo.toml often mentions sister projects. Even if you can't verify a package at runtime, note viable candidates in the findings summary so a human reviewer can evaluate them.

**Direction of sync**: Only sync Rust → {{LANG_NAME}} (add missing features, fix outdated API calls). If the {{LANG_NAME}} CLI has additions not in Rust (e.g., success messages, extra help text, UX improvements), keep them — note them in findings as suggestions for the Rust CLI, but do not remove them.

### Step 3: File mapping

{{FILE_MAPPING}}

`{{SERIALIZATION_FILE}}` is {{LANG_NAME}}-only (Rust uses serde). Don't touch it unless adding a new serialization helper.

**New command subgroup files** (like `contacts.rs`): If the Rust CLI adds a new `command/<name>.rs` subgroup (similar to `issuer.rs`), create a matching {{LANG_NAME}} file at `{{TARGET_DIR}}{{SUBGROUP_PATH_PATTERN}}` following the same pattern as `{{ISSUER_FILE}}`: a dispatch function, a command registry, {{EXTRA_SUBGROUP_COMPONENTS}}

**Platform-specific Rust modules** (like `seedless_restore/`): Some Rust CLI directories contain platform-specific utilities (passkey/FIDO2/YubiKey). If these add new CLI flags or options to `main.rs`, follow the "Before marking anything unsupported" process above — read the Rust code, identify SDK types, implement portable providers fully, and skeleton hardware providers. Always check if the README was also updated and sync documentation accordingly.

### Step 4: Translation rules

{{TRANSLATION_RULES}}

### Step 4b: Documentation sync rules

When `crates/breez-sdk/cli/README.md` changes, update the {{LANG_NAME}} CLI README at
`{{TARGET_DIR}}README.md` to reflect the same
features and configuration options, but with {{LANG_NAME}}-specific syntax and usage:

- **New features/sections**: If the Rust README adds documentation for a new feature
  (e.g., seedless restore, new CLI flags), add an equivalent section to the {{LANG_NAME}} README
  with {{LANG_NAME}}-specific {{DOC_BUILD_INSTRUCTIONS}}.
- **CLI option changes**: If new `--flags` are documented, translate `cargo run -- --flag`
  to `{{CLI_BINARY}} --flag` and update the CLI Options table.
- **Rust-only features**: If a feature is Rust-only (e.g., requires `--features fido2`
  cargo flag, uses YubiKey OTP crates), document it as "Not yet available in {{LANG_NAME}} CLI"
  or skip it if it has no {{LANG_NAME}} equivalent at all.
- **Do NOT copy verbatim**: Rewrite documentation in the style of the existing {{LANG_NAME}}
  README. Keep the existing structure (Prerequisites, Quick Start, Makefile Targets,
  CLI Options, Environment Variables, Available Commands, Development).
- **Preserve existing {{LANG_NAME}}-specific content**: Don't remove the Makefile targets,
  {{DOC_PRESERVE_ITEMS}}.

### Step 5: Write findings summary
After comparing all file pairs, write `sync-findings.md` using this exact format:

```markdown
## Divergences
- [one-line description of each divergence found]

## Applied
- [one-line description of each fix applied, referencing the file changed]

## Skipped
- [one-line description of what was skipped, what protocol/operation was needed, what packages were evaluated, and why none were viable]
```

If no differences were found, write only: `No differences found — CLIs are in sync.`

### Step 6: Scope constraint
ONLY modify files under: `{{TARGET_DIR}}`
Do NOT modify any other files. The only exception is `sync-findings.md`, which must be written to the repository root (not inside `{{TARGET_DIR}}`).

### Step 7: Verify changes
Read back each modified file to verify correctness.

### Step 8: Build check (final gate)
**This must be the very last step.** Do NOT make any code edits after this step passes.
Run the build check to verify the code is syntactically valid and properly formatted:
```bash
{{BUILD_CHECK}}
```
If any check fails, fix the errors and re-run until it passes. {{FORMAT_INSTRUCTIONS}}

### Step 9: No-op check
If the Rust and {{LANG_NAME}} CLIs are already in sync (no meaningful differences), do NOT modify any files. Output: "No {{LANG_NAME}} CLI changes needed."

**Important:** Do NOT create git branches, commits, or pull requests. The CI workflow handles all git operations after you finish. Just leave your changes in the working tree.
