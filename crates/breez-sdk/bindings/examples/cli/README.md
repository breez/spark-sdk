# Breez SDK - CLI Examples

Example CLI clients for the [Breez SDK](../../../../../README.md) in multiple languages.

## Source of Truth

The **[Rust CLI](../../../cli/)** (`crates/breez-sdk/cli/`) is the canonical implementation. All other language CLIs are automated ports that mirror its commands, arguments, and behavior.

When a change is made to the Rust CLI and merged to `main`, the [sync-cli](../../../../../.github/workflows/sync-cli.yml) GitHub Actions workflow automatically detects changes and runs all language syncs in parallel using a matrix strategy. Individual languages can be targeted via `workflow_dispatch` with the `languages` input.

## Available Languages

| Language | Path | Status |
|----------|------|--------|
| [Python](langs/python/) | `langs/python/` | Active |
| [Go](langs/go/) | `langs/go/` | Active |
| [Dart](langs/dart/) | `langs/dart/` | Active |
| [Swift](langs/swift/) | `langs/swift/` | Active |

More languages (Kotlin, WASM, React Native, C#) will be added over time.

## Sync Prompts

The `sync-prompts/` directory contains per-language prompt configs (TOML) and a shared prompt template. The [sync-cli](../../../../../.github/workflows/sync-cli.yml) workflow assembles the final prompt at runtime by rendering the template with language-specific values.

```bash
python3 sync-prompts/generate.py --prompt-only dart   # Preview rendered prompt
python3 sync-prompts/generate.py --list                # List available languages
```

To add a new language, create `sync-prompts/langs/<lang>.toml` (use an existing one as reference) and add a matrix entry in `.github/workflows/sync-cli.yml`.
