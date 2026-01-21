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
# Track review start time for duration calculation
export REVIEW_START_TIME=$(date +%s)

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

# Get linked issues for requirements context
.claude/skills/pr-review/fetch-linked-issues.sh $PR_NUMBER

# Get existing inline comments to avoid duplication
gh api "repos/breez/spark-sdk/pulls/$PR_NUMBER/comments" \
  | jq -r '.[] | "\(.path):\(.line) - \(.body | split("\n")[0] | .[0:80])"' \
  > /tmp/pr-$PR_NUMBER-existing-comments.txt
echo "Existing inline comments saved to /tmp/pr-$PR_NUMBER-existing-comments.txt"

# Get filtered file list (excludes auto-generated files)
.claude/skills/pr-review/filter-diff.sh $PR_NUMBER --names-only

# Get filtered diff
.claude/skills/pr-review/filter-diff.sh $PR_NUMBER
```

Review linked issues and existing comments to avoid duplicating feedback.

## Step 2: Review

**Recommended approach: Use pr-reviewer agent**

Launch the pr-reviewer agent with full context for comprehensive analysis:

```bash
# Agent will receive:
# - Linked issues context (from Step 1)
# - Existing inline comments (from /tmp/pr-$PR_NUMBER-existing-comments.txt)
# - Filtered diff output
# - CI status

# For --post mode, include in prompt:
# "Review PR #$PR_NUMBER with --post mode. Output structured JSON ready for posting."

# For chat mode:
# "Review PR #$PR_NUMBER for potential issues."
```

The agent will:
- Use cached file fetching automatically (`.claude/skills/pr-review/fetch-file.sh`)
- Avoid flagging issues already covered in existing comments
- Validate implementation against linked issue requirements
- Output structured JSON (for --post) or human-readable findings (for chat)

**Alternative: Manual review strategy (for simple PRs)**
1. Review diff output from Step 1 (changed hunks + context)
2. For large PRs, consider chunking: `.claude/skills/pr-review/chunk-diff.sh $PR_NUMBER --max-tokens 20000`
   - Splits diff into manageable chunks by file boundaries
3. For predictable file sets, batch fetch: `.claude/skills/pr-review/batch-fetch-files.sh $PR_NUMBER file1 file2`
   - Efficient parallel fetching with cache statistics
4. For complex changes, use cached file fetching: `.claude/skills/pr-review/fetch-file.sh $PR_NUMBER path/to/file.rs true`
   - Cached fetching eliminates redundant API calls
   - Use `true` flag for line numbers (helpful for inline comment positioning)
5. Only read full files when broader context is required (trait implementations, module structure)

Follow `.claude/agents/pr-reviewer.md` workflow and apply relevant rules from `.claude/rules/`.

## Step 3: Output

**Chat mode (default):**
- Agent returns human-readable findings
- Show summary with severity levels defined in `.claude/skills/pr-review/tone.md`
- Provide clickable GitHub links to code

**Post mode (`--post`):**

If using pr-reviewer agent with structured JSON output:
1. **Extract:** Agent returns JSON with `inline_comments` array ready for posting
2. **Save:** Extract and save to `/tmp/pr-$PR_NUMBER-comments.json`:
   ```bash
   # Extract inline_comments from agent output
   echo "$AGENT_OUTPUT" | jq '.inline_comments' > /tmp/pr-$PR_NUMBER-comments.json
   ```
3. **Post:** Submit using `.claude/skills/pr-review/post-review.sh`:
   ```bash
   # Set token usage for cost calculation (if available from agent)
   # export REVIEW_INPUT_TOKENS=25000
   # export REVIEW_OUTPUT_TOKENS=3000

   .claude/skills/pr-review/post-review.sh \
     $PR_NUMBER \
     "$(echo "$AGENT_OUTPUT" | jq -r '.recommendation')" \
     "$(echo "$AGENT_OUTPUT" | jq -r '.summary')" \
     /tmp/pr-$PR_NUMBER-comments.json \
     "sonnet-4.5"
   ```

   **Note:** Token tracking requires integration with API responses. If tokens are available:
   - Set `REVIEW_INPUT_TOKENS` and `REVIEW_OUTPUT_TOKENS` environment variables
   - Cost and duration will be displayed in CLI (not posted to GitHub)
   - Model attribution footer will be added to GitHub review

If preparing manually:
1. **Prepare:** Create a temporary JSON file with inline comments. **Always use multi-line ranges** (`start_line` + `line`) with 2-4 lines of context. Verify absolute line numbers using `.claude/skills/pr-review/fetch-file.sh $PR_NUMBER path/to/file.rs true` (cached and includes line numbers).
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
