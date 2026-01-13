---
allowed-tools: Bash(gh pr:*), Bash(git:*), Read, Grep, Glob
argument-hint: [pr-number]
description: Review a pull request against repository guidelines
---

## Step 1: Gather PR Context

Determine which PR to review:
- If `$ARGUMENTS` is provided, use that PR number
- Otherwise, detect the PR for the current branch

```bash
# Get PR details
gh pr view $ARGUMENTS --json number,title,body,author,state,additions,deletions,changedFiles

# Get diff (adjust strategy based on size)
gh pr diff $ARGUMENTS

# Check CI status
gh pr checks $ARGUMENTS
```

## Step 2: Review Code Changes

Apply review criteria from `.claude/agents/code-reviewer.md`:
- Design and API decisions (UX-first)
- Security concerns (no keys in logs, checked arithmetic)
- Code quality (no unwrap, doc comments, clippy)
- Binding file consistency (run `validate-bindings.sh` if API changed)

## Step 3: Present Review

Format as a concise review. If posting to GitHub, include:
```markdown
🧪 Experimental PR review using Claude Code.

---

{review_content}
```

## Step 4: Follow-up Actions

If Flutter bindings changed (new features or breaking changes):
1. Check for existing issues: `gh issue list --repo breez/glow --search "{feature}" --state open`
2. Create or update issue using `.claude/skills/pr-review/templates/glow-issue.md`
