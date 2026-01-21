---
allowed-tools: Bash(gh pr:*), Bash(git:*), Bash(.claude/skills/pr-review/*.sh), Read, Grep, Glob
argument-hint: [pr-number] [--post]
description: Review a pull request against repository guidelines
---

## Arguments

- `pr-number` (optional): PR to review, defaults to current branch's PR
- `--post` (optional): Post inline comments to GitHub

Examples:
- `/review` - Review current branch's PR
- `/review 554` - Review PR #554
- `/review 554 --post` - Review and post inline comments

## Step 1: Gather Context

```bash
# Parse arguments
PR_NUMBER=$(echo "$ARGUMENTS" | grep -oE '[0-9]+' | head -1)
POST_MODE=$(echo "$ARGUMENTS" | grep -q '\-\-post' && echo "true" || echo "false")

# Default to current branch's PR
if [ -z "$PR_NUMBER" ]; then
  PR_NUMBER=$(gh pr view --json number -q .number)
fi

# Get PR details, CI status, and existing feedback
gh pr view $PR_NUMBER --json number,title,body,author,state,additions,deletions,changedFiles,headRefName,headRefOid,baseRefName,reviews,comments
gh pr checks $PR_NUMBER

# Get filtered file list (excludes auto-generated files)
.claude/skills/pr-review/filter-diff.sh $PR_NUMBER --names-only

# Get filtered diff
.claude/skills/pr-review/filter-diff.sh $PR_NUMBER
```

Check existing discussions to avoid duplicating feedback.

## Step 2: Review

**Token-efficient strategy:**
1. Review diff output from Step 1 (changed hunks + context)
2. For complex changes, fetch more context: `gh pr diff $PR_NUMBER --unified=20 -- path/to/file.rs`
3. Only read full files when broader context is required (trait implementations, module structure)

Follow `.claude/agents/pr-reviewer.md` workflow and apply relevant rules from `.claude/rules/`.

## Step 3: Output

**Chat mode (default):** Show summary findings with severity levels defined in `.claude/skills/pr-review/tone.md`. Provide clickable GitHub links to code.

**Post mode (`--post`):**
1. **Prepare:** Create a temporary JSON file with inline comments. **Always use multi-line ranges** (`start_line` + `line`) with 2-4 lines of context. Verify absolute line numbers by fetching actual file content with `gh api`.
2. **Post:** Submit findings using `.claude/skills/pr-review/post-review.sh`.
3. **Reference:** Full API and range details in `.claude/skills/pr-review/github-inline-comments.md`.

## Follow-up

For binding changes, check if downstream integration is needed:
```bash
# Flutter → Glow app
gh issue list --repo breez/glow --search "{feature}" --state open

# WASM → Web example
gh issue list --repo breez/breez-sdk-spark-example --search "{feature}" --state open
```

Use `.claude/skills/pr-review/templates/follow-up-issue.md` template if creating issues.
