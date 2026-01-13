---
name: pr-review
description: Review code changes against Breez SDK guidelines. Use when reviewing PRs or significant code changes.
---

# PR Review Skill

This skill provides the `/review` command and supporting templates for PR reviews.

## Usage

```
/review [pr-number]
```

The command uses review criteria from `.claude/agents/code-reviewer.md`.

## Templates

- `templates/glow-issue.md` - Follow-up issue format for breez/glow

## Scripts

- `validate-bindings.sh` - Check binding file consistency for API changes
