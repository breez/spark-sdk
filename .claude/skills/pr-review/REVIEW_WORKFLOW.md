# Complete Review Workflow with Duplicate Prevention

This is the recommended workflow for reviewing PRs safely and efficiently.

## Step 0: Check Existing Reviews (CRITICAL - Do This First!)

**Always check for existing reviews before spending time on a detailed review:**

```bash
./.claude/skills/pr-review/scripts/analysis/analyze-review-status.sh <pr-number>
```

This tells you:
- If the PR code has changed since the last review
- What issues are already flagged
- Whether to proceed with a new review or just add comments

**Decision Matrix:**

| Code Changed? | Existing Review? | Action |
|---------------|-----------------|--------|
| No | No | ✅ Proceed with new review |
| No | Yes | ⏸️  STOP - Don't duplicate. Use `.claude/skills/pr-review/scripts/posting/add-review-comment.sh` if you found new issues |
| Yes | No | ✅ Proceed with new review |
| Yes | Yes | ✅ Re-review to check which issues are fixed and find new ones |

## Step 1: Review the Code

Review according to `.claude/agents/pr-reviewer.md` and apply relevant rules from `.claude/rules/`.

Use efficient diff-based review (see `.claude/skills/pr-review/docs/diff-workflow.md`):
```bash
# Get filtered diff
./.claude/skills/pr-review/scripts/fetching/filter-diff.sh <pr-number>

# Get more context for complex sections
gh pr diff <pr-number> -- path/to/file.rs
```

## Step 2: Prepare Comments (If Posting)

### Option A: Generate and Post Individual Comments

```bash
# Generate a single comment
./.claude/skills/pr-review/scripts/generation/generate-comment.sh \
  "crates/sdk/src/lib.rs" \
  42 \
  45 \
  "Blocking" \
  "SQL injection - use parameterized queries"

# Combine multiple comments into array
jq -s '. | add' /tmp/c1.json /tmp/c2.json > comments.json
```

### Option B: Batch Generate from Text File

Create `comments.txt`:
```
crates/sdk/src/lib.rs:42:45:Blocking:SQL injection vulnerability
crates/sdk/src/lib.rs:100:105:Important:Missing error handling
```

Generate JSON array:
```bash
./.claude/skills/pr-review/scripts/generation/build-comments.sh < comments.txt > comments.json
```

## Step 3: Post the Review

```bash
./.claude/skills/pr-review/scripts/posting/post-review.sh \
  <pr-number> \
  REQUEST_CHANGES \
  "Summary of findings" \
  comments.json
```

## Adding Follow-Up Comments (Code Unchanged)

If code hasn't changed but you found new issues or want to add details:

```bash
# Add a comment instead of a duplicate review
./.claude/skills/pr-review/scripts/posting/add-review-comment.sh \
  <pr-number> \
  COMMENT \
  "New finding: Missing validation on line 42"

# Or for blocking feedback
./.claude/skills/pr-review/scripts/posting/add-review-comment.sh \
  <pr-number> \
  REQUEST_CHANGES \
  "Critical issue found in latest inspection: X is not thread-safe"
```

## Checking on Progress

If a PR has been waiting for fixes:

```bash
# Check status again
./.claude/skills/pr-review/scripts/analysis/analyze-review-status.sh <pr-number>

# If still no changes after reasonable time, add a follow-up
./.claude/skills/pr-review/scripts/posting/add-review-comment.sh \
  <pr-number> \
  COMMENT \
  "Checking on status of fixes. Are you still working on these issues?"
```

## Summary

**The critical step most people miss:** Always run `analyze-review-status.sh` FIRST.

This prevents:
- ❌ Posting duplicate REQUEST_CHANGES reviews
- ❌ Commenting on already-flagged issues without new insights
- ❌ Wasting time reviewing unchanged code
- ❌ Misunderstanding which issues have been fixed

**Expected workflow for most reviews:**

```bash
# 1. CHECK FIRST (prevents duplicates)
./.claude/skills/pr-review/analyze-review-status.sh 569

# 2. Decision based on output
# ... if safe to proceed:

# 3. Review and prepare comments
# ... generate comments.json ...

# 4. Post
./.claude/skills/pr-review/scripts/posting/post-review.sh 569 REQUEST_CHANGES "Summary" comments.json
```
