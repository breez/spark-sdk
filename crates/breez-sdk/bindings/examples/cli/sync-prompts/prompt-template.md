## Task: Sync {{LANG_NAME}} CLI with Rust CLI changes

Your job is to update the {{LANG_NAME}} CLI to match the current Rust CLI.

### What changed
${{ steps.diff-info.outputs.diff_summary }}

### Step 1: Analyze the changes
If a diff base was provided, run: `git diff ${{ steps.diff-info.outputs.diff_base }} HEAD -- 'crates/breez-sdk/cli/src/' 'crates/breez-sdk/cli/README.md'`
Otherwise (manual trigger without a base SHA), read the full Rust CLI source files and compare them against the current {{LANG_NAME}} CLI to identify all differences.
Always read the current Rust files for full context.

### Step 2: File mapping

{{FILE_MAPPING}}

`{{SERIALIZATION_FILE}}` is {{LANG_NAME}}-only (Rust uses serde). Don't touch it unless adding a new serialization helper.

**New command subgroup files** (like `contacts.rs`): If the Rust CLI adds a new `command/<name>.rs` subgroup (similar to `issuer.rs`), create a matching {{LANG_NAME}} file at `{{TARGET_DIR}}{{SUBGROUP_PATH_PATTERN}}` following the same pattern as `{{ISSUER_FILE}}`: a dispatch function, a command registry, {{EXTRA_SUBGROUP_COMPONENTS}}

**Platform-specific Rust modules** (like `seedless_restore/`): Some Rust CLI directories contain platform-specific utilities (passkey/FIDO2/YubiKey). If these add new CLI flags or options to `main.rs`, implement the {{LANG_NAME}} equivalent where feasible. If a feature depends on Rust-only crates with no {{LANG_NAME}} equivalent, add the CLI flags but {{UNSUPPORTED_HANDLER}}. Always check if the README was also updated and sync documentation accordingly.

### Step 3: Translation rules

{{TRANSLATION_RULES}}

### Step 3b: Documentation sync rules

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

### Step 4: Scope constraint
ONLY modify files under: `{{TARGET_DIR}}`
Do NOT modify any other files.

### Step 5: Build check
After making changes, verify the code is syntactically valid:
```bash
{{BUILD_CHECK}}
```
If any check fails, fix the errors before proceeding.

### Step 6: Verify and create PR
After making changes:
1. Read back each modified file to verify correctness
2. **If this is a dry run** (`${{ inputs.dry-run }}` is `true`): do NOT create a branch or PR. Instead, output a summary of all changes you made (files modified, what changed in each) and end with: "Dry run complete — no PR created."
3. **Otherwise**, create a branch and PR:
```bash
git checkout -b claude/sync-{{LANG_ID}}-cli-$(echo "${{ github.sha }}" | cut -c1-7)
git add {{TARGET_DIR}}
git commit -m "chore: sync {{LANG_NAME}} CLI with Rust CLI changes (${{ github.sha }})"
git push -u origin HEAD
gh pr create --title "chore: sync {{LANG_NAME}} CLI with Rust CLI changes" \
  --body "Automated sync of {{LANG_NAME}} CLI from Rust CLI changes in ${{ github.sha }}" \
  --base main
```

### Step 7: No-op check
If the Rust changes don't affect CLI commands or documentation (e.g., only Cargo.toml changes or internal-only refactoring with no user-facing impact), do NOT create a PR. Output: "No {{LANG_NAME}} CLI changes needed."
