---
allowed-tools: Bash(gh pr:*), Bash(git:*), Read, Grep, Glob
argument-hint: [pr-number] [--post]
description: Review a pull request against repository guidelines
---

## Arguments

- `pr-number` (optional): PR number to review, defaults to current branch's PR
- `--post` (optional): Post inline review comments to GitHub instead of chat-only

Examples:
- `/review` - Review current branch's PR in chat
- `/review 554` - Review PR #554 in chat
- `/review 554 --post` - Review PR #554 and post inline comments to GitHub

## Step 1: Gather PR Context

Parse arguments:
```bash
# Extract PR number and mode
PR_NUMBER=$(echo "$ARGUMENTS" | grep -oE '[0-9]+' | head -1)
POST_MODE=$(echo "$ARGUMENTS" | grep -q '\-\-post' && echo "true" || echo "false")

# If no PR number, detect from current branch
if [ -z "$PR_NUMBER" ]; then
  PR_NUMBER=$(gh pr view --json number -q .number)
fi
```

Determine which PR to review and output mode:
- If `--post` flag present, post inline comments to GitHub
- Otherwise, show review in chat with clickable links

```bash
# Get PR details
gh pr view $ARGUMENTS --json number,title,body,author,state,additions,deletions,changedFiles

# Get diff (adjust strategy based on size)
gh pr diff $ARGUMENTS

# Check CI status
gh pr checks $ARGUMENTS

# Get existing comments and discussions
gh api repos/{owner}/{repo}/pulls/{number}/comments
gh pr view $ARGUMENTS --json reviews,comments
```

Review existing discussions before providing feedback:
- Don't repeat points others already raised
- Note if author addressed concerns in responses
- Build on existing suggestions rather than duplicate them

## Step 2: Review Code Changes

Apply review criteria from `.claude/agents/code-reviewer.md`:
- Design and API decisions (UX-first)
- Security concerns (no keys in logs, checked arithmetic)
- Code quality (no unwrap, doc comments, clippy)
- Binding file consistency (run `validate-bindings.sh` if API changed)

Identify which context-dependent checks apply based on changed files:
- **Core models** (`models/`): UniFFI macros, serde derives, adaptors
- **SDK interface** (`sdk.rs`): Binding consistency across WASM/Flutter
- **CLI** (`cli/`): Command-to-SDK mapping, argument naming
- **Documentation** (`snippets/`): Parallel language examples

## Step 3: Present Review

**Check `POST_MODE` variable:**

**If `POST_MODE=false` (default):**
- Show review in chat with clickable GitHub links
- No changes posted to GitHub

**If `POST_MODE=true` (via `--post` flag):**
- Use `gh pr review $PR_NUMBER --comment` for each issue
- Add header: `> 🧪 Experimental PR review using Claude Code.`
- Post issues as review comments using `--path` and `--line` flags
- Example:
```bash
gh pr review $PR_NUMBER --comment \
  --body "**[CRITICAL]** Issue description

**Fix:** Suggested resolution" \
  --path "path/file.rs" \
  --line 42
```
- See `.claude/agents/code-reviewer.md` for complete inline comment format

## Step 4: Follow-up Actions

If Flutter bindings changed (new features or breaking changes):
1. Check for existing issues: `gh issue list --repo breez/glow --search "{feature}" --state open`
2. Create or update issue using `.claude/skills/pr-review/templates/glow-issue.md`
