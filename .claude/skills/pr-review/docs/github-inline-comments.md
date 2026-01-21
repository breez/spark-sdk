# GitHub Inline Comments API

## Preventing Duplicate Reviews

**Always check for existing reviews before posting a new one:**

```bash
# Analyze existing reviews and show what's already been flagged
./.claude/skills/pr-review/analyze-review-status.sh <pr-number>
```

This will tell you:
- If the code has been updated since the last review
- Which issues are still present vs potentially fixed
- Whether to post a new review, add a comment, or wait

See `.claude/skills/pr-review/REVIEW_WORKFLOW.md` for decision matrix and next steps.

**If duplicate issues are found:**
- Use `.claude/skills/pr-review/add-review-comment.sh` to add new findings to existing review
- Or use `.claude/skills/pr-review/post-review.sh` if the review author has updated their code

## Workflow

Posting a review is a **three-step process**:
1. **Check** for existing reviews with `analyze-review-status.sh`
2. **Generate** inline comments using helper scripts (handles all formatting automatically)
3. **Submit** the review using `post-review.sh` (only if new issues found)

## Quick Start

### Option A: Command-by-Command (Simple, Interactive)

For a single comment:
```bash
# Generate one comment and save to file
./.claude/skills/pr-review/generate-comment.sh \
  "crates/sdk/src/lib.rs" \
  42 \
  45 \
  "Blocking" \
  "SQL injection vulnerability in query builder" > /tmp/comment.json

# Create array with one comment
jq -s '.' /tmp/comment.json > comments.json
```

For multiple comments:
```bash
# Generate each comment to a temp file
./.claude/skills/pr-review/generate-comment.sh "crates/sdk/src/lib.rs" 42 45 "Blocking" "First issue" > /tmp/c1.json
./.claude/skills/pr-review/generate-comment.sh "crates/sdk/src/lib.rs" 100 105 "Important" "Second issue" > /tmp/c2.json

# Combine into array
jq -s '. | add' /tmp/c1.json /tmp/c2.json > comments.json
```

### Option B: Batch Mode (Recommended for Multiple Comments)

Create a simple text file with one comment per line:
```
# comments.txt
crates/sdk/src/lib.rs:42:45:Blocking:SQL injection - use parameterized queries
crates/sdk/src/lib.rs:100:105:Important:Error handling missing on line 102
crates/sdk/src/handler.rs:15:20:Suggestion:Consider extracting this logic to a helper
```

Generate the JSON array automatically:
```bash
./.claude/skills/pr-review/build-comments.sh < comments.txt > comments.json
```

### Step 3: Post Review

```bash
./.claude/skills/pr-review/post-review.sh 569 REQUEST_CHANGES "Summary of findings" comments.json
```

## How to Find Line Numbers

The helper scripts handle all JSON formatting, but you still need correct line numbers. To find them:

1. **From PR diff**: Look at the line numbers in the diff header (e.g., `@@ -42,10 +45,15 @@`)
2. **From actual file on PR branch**: Use `git show FETCH_HEAD:path/to/file.rs | grep -n "search_term"` to find exact line numbers
3. **Verify with context**: Use `git show FETCH_HEAD:path/to/file.rs | sed -n '40,50p' | cat -n` to verify surrounding lines

## Helper Scripts Reference

### analyze-review-status.sh

Checks existing reviews and determines if the PR code has been updated since the last review.

```bash
./.claude/skills/pr-review/analyze-review-status.sh <pr-number> [review-commit-sha]

# Outputs:
#   - Current commit vs review commit
#   - Extracted issues from previous review
#   - Inline comments from that review
#   - Recommendations on next action
```

**Determines if:**
- Code hasn't changed since review (issues still present)
- Code has changed (need to re-verify which issues are fixed)
- New issues might exist

### add-review-comment.sh

Add a follow-up comment to a PR instead of posting a duplicate review.

```bash
./.claude/skills/pr-review/add-review-comment.sh <pr-number> <event> <message>

# Arguments:
#   pr-number - PR to comment on
#   event     - COMMENT (non-blocking) or REQUEST_CHANGES (blocking)
#   message   - Your comment text

# Example:
./.claude/skills/pr-review/add-review-comment.sh 569 COMMENT "Issue #1 still present in latest commit"
./.claude/skills/pr-review/add-review-comment.sh 569 COMMENT "New finding: Missing validation on line 42"
```

**Use when:**
- Previous review exists and code hasn't fully fixed all issues
- You found new issues the previous review missed
- You want to add details/clarification without creating a duplicate review

### generate-comment.sh

Generates a single JSON comment object with automatic validation and formatting.

```bash
./.claude/skills/pr-review/generate-comment.sh <path> <start> <end> <severity> <body>

# Arguments:
#   path      - File path relative to repo root (required)
#   start     - First line number (required, positive integer)
#   end       - Last line number (required, must be > start)
#   severity  - One of: Blocking, Important, Suggestion (required)
#   body      - Comment text (required, multi-line OK)

# Example:
./.claude/skills/pr-review/generate-comment.sh \
  "crates/sdk/src/lib.rs" \
  42 \
  45 \
  "Blocking" \
  "SQL injection - use parameterized queries instead of format!"
```

### build-comments.sh

Generates a complete JSON array from a batch of comments in text format.

```bash
# Input format: path:start:end:severity:body (one per line)
./.claude/skills/pr-review/build-comments.sh < comments.txt > comments.json

# Example input file:
# crates/sdk/src/lib.rs:42:45:Blocking:SQL injection vulnerability
# crates/sdk/src/lib.rs:100:105:Important:Missing error handling
```

## JSON Format (For Reference)

Each comment is automatically formatted as:
```json
{
  "path": "file/path.rs",
  "start_line": 42,
  "line": 45,
  "side": "RIGHT",
  "start_side": "RIGHT",
  "body": "**Blocking** - Comment text"
}
```

**What gets validated**:
- `path`: Non-empty string
- `start_line`: Positive integer, must be < `line`
- `line`: Positive integer, must be > `start_line`
- `side`, `start_side`: Must be "RIGHT"
- `body`: Non-empty string

**This is all handled by the helper scripts** - you only provide: path, line numbers, severity, and comment text.

For detailed workflow guidance, see `.claude/skills/pr-review/REVIEW_WORKFLOW.md`.

## Severity Levels

Always start your comment body with one of these prefixes:

| Level | Meaning | Example |
|-------|---------|---------|
| **Blocking** | Prevents merge | Security vulnerability, breaking change |
| **Important** | Should be addressed | Bug, significant improvement needed |
| **Suggestion** | Nice to have | Style preference, minor optimization |

## Review Limitations Disclaimer

When non-ignored files exceed the token limit (25,000 tokens), include a disclaimer in the review summary:

```markdown
---

> ⚠️ **Review limitation:** The following files exceeded token limits and were not fully reviewed:
> - `path/to/large-file.rs` (truncated)
```

This helps identify files that may need splitting or added to `.claude/skills/pr-review/filter-diff.sh` ignore patterns.
