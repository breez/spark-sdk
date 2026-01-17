---
description: Update documentation snippets across all 9 languages
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, Task, TodoWrite
---

# Update Documentation Snippets

Update code snippets in `docs/breez-sdk/snippets/` across all supported languages.

## Arguments

The user should provide:
1. **Snippet file name(s)** - one or more (e.g., `send_payment`, `receive_payment`)
2. **Change description** (what to add/modify)

### Multiple Files

When updating multiple snippet files:
- Process **one file completely** (all 9 languages) before moving to next file
- This prevents partial updates scattered across files if something fails
- Create a todo list with files as top-level items, languages nested conceptually

## Workflow

### Phase 1: Setup

1. Read `docs/breez-sdk/snippets/SNIPPET_CONVENTIONS.md` for patterns
2. Create a todo list with all 9 languages

### Phase 2: Update Rust (Canonical Reference)

1. Read the Rust snippet file
2. Make the requested changes following conventions
3. Verify: `cargo xtask check-doc-snippets --package rust --skip-build`
4. If verification fails, fix and re-verify
5. Mark Rust as complete

### Phase 3: Update All Other Languages (Parallel)

**Spawn all 8 agents in a single message** for: `go`, `python`, `kotlin-mpp`, `swift`, `csharp`, `flutter`, `wasm`, `react-native`

Use this prompt template for each:

```
You are updating the {LANGUAGE} snippet to match Rust.

## Reference (Rust - canonical)
{RUST_SNIPPET_CONTENT}

## Conventions
Read: docs/breez-sdk/snippets/SNIPPET_CONVENTIONS.md
Focus on the {LANGUAGE} patterns for: imports, function signatures, enum discrimination, logging, error handling.

## Your Task
1. Read the current {LANGUAGE} snippet at: {LANGUAGE_FILE_PATH}
2. Compare with Rust reference - identify what changed
3. Apply equivalent changes following {LANGUAGE}-specific conventions
4. Ensure ANCHOR names match Rust exactly (kebab-case)
5. Ensure comments/descriptions match semantically
6. Use the Edit tool to make changes (or Write if new file)

Do NOT run verification - the main agent will handle that.
```

### Phase 4: Verify All Languages

After all agents complete, run verifications. Can be parallel:

```bash
# Run all verifications (parallel in background or sequential):
cargo xtask check-doc-snippets --package go --skip-build &
cargo xtask check-doc-snippets --package python --skip-build &
cargo xtask check-doc-snippets --package kotlin-mpp --skip-build &
cargo xtask check-doc-snippets --package swift --skip-build &
cargo xtask check-doc-snippets --package csharp --skip-build &
cargo xtask check-doc-snippets --package flutter --skip-build &
cargo xtask check-doc-snippets --package wasm --skip-build &
cargo xtask check-doc-snippets --package react-native --skip-build &
wait
```

If any verification fails:
1. Fix the specific language
2. Re-verify just that language
3. Continue until all pass

### Phase 5: Summary

Report:
- Languages updated successfully
- Any issues encountered
- Verification status for each language

## File Paths

| Language | Path Pattern |
|----------|-------------|
| Rust | `docs/breez-sdk/snippets/rust/src/{file}.rs` |
| Go | `docs/breez-sdk/snippets/go/{file}.go` |
| Python | `docs/breez-sdk/snippets/python/src/{file}.py` |
| Kotlin | `docs/breez-sdk/snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/{File}.kt` |
| Swift | `docs/breez-sdk/snippets/swift/BreezSdkSnippets/Sources/{File}.swift` |
| C# | `docs/breez-sdk/snippets/csharp/{File}.cs` |
| Flutter | `docs/breez-sdk/snippets/flutter/lib/{file}.dart` |
| WASM | `docs/breez-sdk/snippets/wasm/{file}.ts` |
| React Native | `docs/breez-sdk/snippets/react-native/{file}.ts` |

Note: Some languages use PascalCase filenames (Kotlin, Swift, C#).

## Verification Commands

**Node.js Requirement:** WASM and React Native require Node >= 22. Before verifying those:
```bash
# Check and set Node version if nvm is available:
node --version || true
command -v nvm && nvm use 22 || true
```

```bash
# Individual language (fast with --skip-build):
cargo xtask check-doc-snippets --package rust --skip-build
cargo xtask check-doc-snippets --package go --skip-build
cargo xtask check-doc-snippets --package python --skip-build
cargo xtask check-doc-snippets --package kotlin-mpp --skip-build
cargo xtask check-doc-snippets --package swift --skip-build
cargo xtask check-doc-snippets --package csharp --skip-build
cargo xtask check-doc-snippets --package flutter --skip-build
cargo xtask check-doc-snippets --package wasm --skip-build
cargo xtask check-doc-snippets --package react-native --skip-build

# First run after SDK interface changes (rebuilds bindings):
cargo xtask check-doc-snippets --package {language}
```

## Change Types

### Modifying Existing Snippets
- Read current snippet, apply changes, verify
- Sub-agent receives both Rust reference AND current language snippet for context

### Adding New Functions
- Add to Rust first with proper ANCHOR markers
- Sub-agents translate the new function following conventions
- Ensure ANCHOR name is kebab-case and identical across all languages

### Adding New Snippet Files
- Create Rust file first with all functions and ANCHORs
- Create each language file following the import/structure patterns in conventions
- May need to update build files (Cargo.toml, go.mod, etc.) - verify will catch this

## Important Notes

- **ANCHOR names must be identical** across all languages
- **Comments should match semantically** (same meaning, language-appropriate syntax)
- **Rust first, then parallel** - Rust must complete before spawning other agents
- **Single message for parallel agents** - spawn all 8 in one tool call block
- **Verify after all agents complete** - not interleaved with agent spawning
- If `--skip-build` fails with missing types, run without it once to rebuild bindings
