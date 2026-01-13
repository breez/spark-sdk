---
allowed-tools: Bash(gh pr:*), Bash(git:*), Task, Read, Grep, Glob
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

Use the Task tool with `subagent_type: "code-reviewer"` to perform a thorough code review.

Pass the PR details and diff to the agent. The code-reviewer agent will analyze:
- Design and API decisions
- Security concerns
- Code quality
- Binding file consistency

## Step 3: Present Review

Format the agent's analysis as a concise review.

If posting to GitHub, include:
```markdown
🧪 Experimental PR review using Claude Code.

---

{review_content}
```

## Step 4: Handle Follow-ups (if applicable)

If the PR includes Flutter binding changes (new features or breaking changes):

1. Check for existing Glow issues:
   ```bash
   gh issue list --repo breez/glow --search "{feature}" --state open
   ```

2. Create or update using template from `.claude/skills/pr-review/templates/glow-issue.md`
