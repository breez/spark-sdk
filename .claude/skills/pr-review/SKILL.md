---
name: pr-review
description: Review code changes against Breez SDK guidelines. Use when reviewing PRs or significant code changes.
---

# PR Review Skill

This skill provides templates and guidance for PR reviews.

## Usage

Use the `code-reviewer` agent to perform thorough code reviews. The agent contains all review criteria for:
- Design evaluation (UX-first API design)
- Security checks (key handling, crypto ops)
- Code quality (no unwrap, doc comments, clippy)
- Binding consistency (all 5 files updated)

## Templates

### Glow Follow-up Issues

When a PR adds new Flutter binding features or breaking changes, create a follow-up issue on [breez/glow](https://github.com/breez/glow).

**Important**: Check for existing issues first to avoid duplicates:
```bash
gh issue list --repo breez/glow --search "spark-sdk" --state open
```

Template: `templates/glow-issue.md`

## Review Workflow

1. Gather PR context (metadata, diff, CI status)
2. Delegate to `code-reviewer` agent for analysis
3. Present concise review
4. If Flutter bindings changed, handle Glow follow-up (create or update)
