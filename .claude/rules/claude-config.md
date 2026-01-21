---
globs: ".claude/**/*"
---
# Claude Configuration Language Guide

This guide defines the language patterns for Claude configuration files in this repository. Follow these patterns when creating or modifying files in `.claude/`.

## Core Principles

### 1. Be Direct, Not Emphatic

Provide context that makes importance clear rather than using emphasis words.

| Avoid | Prefer |
|-------|--------|
| `CRITICAL: Always check...` | `Check X because Y can cause Z.` |
| `NEVER use unwrap` | `Use Result handling - unwrap can panic in production.` |
| `MUST be updated together` | `Update together to prevent runtime binding mismatches.` |
| `IMPORTANT: Do not skip` | `Skipping this causes build failures in CI.` |

### 2. Positive Framing

Tell what to do instead of what not to do.

| Avoid | Prefer |
|-------|--------|
| `Don't log secrets` | `Keep logs free of secrets (keys, tokens, seeds).` |
| `Never skip validation` | `Validate all external input before processing.` |
| `Avoid floating-point` | `Use integer satoshis for monetary amounts.` |

### 3. Add Context (The "Why")

Instructions with rationale are followed more reliably.

| Without Context | With Context |
|-----------------|--------------|
| `Run make check` | `Run make check before committing to catch formatting and lint issues.` |
| `Add UniFFI macros` | `Add UniFFI macros so types are available in language bindings.` |
| `Use checked arithmetic` | `Use checked arithmetic to catch overflow in amount calculations.` |

### 4. Structured Formats

Use tables for mappings and references. Use consistent heading hierarchy.

```markdown
## Section (##)
### Subsection (###)

| Column A | Column B |
|----------|----------|
| Value 1 | Value 2 |
```

### 5. Conciseness

Remove filler words and redundant phrases.

| Verbose | Concise |
|---------|---------|
| `You should make sure to always...` | `Always...` |
| `It is important to note that...` | (just state the fact) |
| `Please ensure that you...` | (just state the instruction) |
| `In order to...` | `To...` |

### 6. Cross-References

Always use full paths from repo root, even for files in the same directory.

| Incorrect | Correct |
|-----------|---------|
| `See tone.md for feedback style.` | `See `.claude/skills/pr-review/tone.md` for feedback style.` |
| `Read the workflow guide` | `See `.claude/skills/pr-review/REVIEW_WORKFLOW.md` for the complete workflow.` |

For repo root files, prefix with `./`:
- `./CLAUDE.md` - project instructions
- `./Makefile` - build commands

Why: Full paths make documentation self-contained and searchable. Readers can find referenced files even outside the current directory context.

## File Structure

### Rules Files

Rules files use YAML frontmatter for path patterns:

```markdown
---
globs: "path/pattern/**/*"
---
# Rule Title

Content...
```

### Agent Files

Agent files use YAML frontmatter for metadata:

```markdown
---
name: agent-name
description: Brief description
skills: skill-name
tools: Tool1, Tool2
---

Content...
```

## Token Efficiency

- Reference other files instead of duplicating content
- Use tables for structured data (more compact than prose)
- Keep rule files focused on one concern
- Load context conditionally via path patterns
