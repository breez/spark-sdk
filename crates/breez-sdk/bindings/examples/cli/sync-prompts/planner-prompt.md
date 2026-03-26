## Task: Plan which language CLIs need syncing with the Rust CLI

You are the **planner** for the CLI sync workflow. Your job is to analyze what changed in the Rust CLI and determine which language CLIs need updating.

### What changed in the Rust CLI
{{DIFF_SUMMARY}}

### Language CLIs to evaluate

{{LANGUAGE_SUMMARY}}

### Instructions

1. **Understand the Rust CLI changes.** If a diff base was provided, run:
   ```bash
   git diff {{DIFF_BASE}} HEAD -- 'crates/breez-sdk/cli/src/' 'crates/breez-sdk/cli/README.md'
   ```
   Read the diff carefully to understand what was added, removed, or modified.

2. **Identify what needs to sync.** From the diff, extract:
   - New commands or subcommands added
   - New CLI flags or options
   - Changed SDK method calls (renamed, different arguments, new calls)
   - README changes (new sections, updated docs)
   - Removed or deprecated features

3. **Check each language CLI.** For each language, quickly check if the target files already have the equivalent changes:
   - Grep for new command names, function names, or flag names in the target directory
   - Read the relevant target file(s) if a grep match is ambiguous
   - If the language already has the equivalent code, mark it as "skip"

4. **Write the plan.** Create a file called `sync-plan.json` in the repository root with this exact JSON structure:

```json
{
  "languages": ["lang-id-1", "lang-id-2"],
  "per_language": {
    "lang-id-1": {
      "files_to_change": ["TargetFile1.ext", "TargetFile2.ext"],
      "guidance": "One-paragraph description of what needs to change and how."
    }
  },
  "skipped": {
    "lang-id-3": "Brief reason why this language was skipped"
  }
}
```

**Rules:**
- `languages` must be a JSON array of language IDs that need syncing (use the exact IDs from the table above)
- `per_language` must have an entry for every language in the `languages` array
- `skipped` should list every language NOT in `languages` with a brief reason
- `guidance` should reference specific patterns from the language's existing code (e.g., "follow the pattern in Issuer.kt")
- If NO languages need syncing, set `languages` to an empty array `[]`
- Be conservative: if you're unsure whether a language needs syncing, include it

**Important:** Only write `sync-plan.json`. Do NOT modify any other files.
