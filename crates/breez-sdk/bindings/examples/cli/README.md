# Breez SDK - CLI Examples

Example CLI clients for the [Breez SDK](../../../../../README.md) in multiple languages.

## Source of Truth

The **[Rust CLI](../../../cli/)** (`crates/breez-sdk/cli/`) is the canonical implementation. All other language CLIs are automated ports that mirror its commands, arguments, and behavior.

When a change is made to the Rust CLI and merged to `main`, the [sync-cli-langs](../../../../../.github/workflows/sync-cli-langs.yml) GitHub Actions workflow automatically detects changes and opens PRs to update each language CLI in parallel. Individual language sync workflows ([Python](../../../../../.github/workflows/sync-python-cli.yml), [Go](../../../../../.github/workflows/sync-go-cli.yml), [Dart](../../../../../.github/workflows/sync-dart-cli.yml)) can also be triggered manually for targeted retries.

## Available Languages

| Language | Path | Status |
|----------|------|--------|
| [Python](langs/python/) | `langs/python/` | Active |
| [Go](langs/go/) | `langs/go/` | Active |
| [Dart](langs/dart/) | `langs/dart/` | Active |

More languages (Swift, Kotlin, WASM, React Native, C#) will be added over time.

## Sync Prompts

The `sync-prompts/` directory contains a template-based generator that produces per-language GitHub Actions workflow YAML. Each language is defined by a TOML config under `sync-prompts/langs/`.

```bash
make sync-prompts-check      # Verify generated workflows are up-to-date
make sync-prompts-generate    # Regenerate workflows from templates
```

To add a new language, create `sync-prompts/langs/<lang>.toml` (use an existing one as reference) and run `make sync-prompts-generate`.
