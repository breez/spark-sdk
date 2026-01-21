# .claude/ Directory

Configuration for Claude Code in this repository.

## Structure

```
.claude/
├── rules/          # Context loaded based on file path globs
├── commands/       # User-invocable slash commands (/update-snippets)
├── skills/         # Reusable capabilities (loaded by agents or commands)
├── agents/         # Specialized agents for Task tool
├── hooks/          # Shell scripts triggered by events
├── settings.json   # Shared settings (committed)
└── settings.local.json  # Local settings (gitignored)
```

## Adding New Files

### Rules

Rules provide context when working with specific file paths.

1. Create `.claude/rules/<name>.md`
2. Add frontmatter with glob pattern:
   ```yaml
   ---
   globs: "path/pattern/**/*"
   ---
   ```
3. Follow patterns in `rules/claude-config.md`

### Commands

Commands are user-invocable via `/<command-name>`.

1. Create `.claude/commands/<name>.md`
2. Add frontmatter:
   ```yaml
   ---
   description: Short description
   allowed-tools: Tool1, Tool2
   ---
   ```

### Skills

Skills are reusable capabilities loaded by agents or commands.

1. Create `.claude/skills/<skill-name>/SKILL.md`
2. Add supporting files in the same directory
3. Reference from agents or commands

### Agents

Agents are specialized for the Task tool.

1. Create `.claude/agents/<name>.md`
2. Add frontmatter:
   ```yaml
   ---
   name: agent-name
   description: Brief description
   tools: Tool1, Tool2
   ---
   ```

## Path-Specific Rules

These conditional rules only apply when Claude is working with files matching the specified patterns.

| Rule | Applies To |
|------|------------|
| `rust.md` | `{crates,packages/flutter/rust}/**/*.rs` |
| `wasm.md` | `**/wasm/**/*` |
| `bindings.md` | `crates/breez-sdk/bindings/**/*`, `packages/**/*` |
| `security.md` | `**/signer/**/*`, `**/token/**/*`, `crates/spark-wallet/**/*` |
| `documentation.md` | `docs/**/*`, `**/*.md` |
| `claude-config.md` | `.claude/**/*` |

Rules without a `paths` field are loaded unconditionally and apply to all files.

## Scripts

Shell scripts should place configurable values at the top:

```bash
#!/bin/bash
# Description of what this script does
# Usage: script.sh <args>

# ==============================================================================
# CONFIGURATION - Edit values here
# ==============================================================================
CONFIG_VALUE="default"
# ==============================================================================

# ... rest of script
```

## Cross-References

Always use full paths from repo root, even for files in the same directory:

```markdown
<!-- Correct -->
See `.claude/skills/pr-review/tone.md` for feedback style.

<!-- Incorrect -->
See `tone.md` for feedback style.
```

For repo root files, prefix with `./`:
- `./CLAUDE.md`

This ensures references remain unambiguous and searchable. Check references still exist when renaming or removing files.
