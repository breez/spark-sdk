---
name: pr-review
description: Review code changes against Breez SDK guidelines. Use when reviewing PRs or significant code changes.
---

# PR Review Skill

Provides the `/review` command and supporting templates for PR reviews.

## Usage

```
/review [pr-number] [--post]
```

⚠️ **IMPORTANT:** Before posting a review, always check for existing reviews:
```bash
./.claude/skills/pr-review/scripts/analysis/analyze-review-status.sh <pr-number>
```

This prevents posting duplicate reviews when code hasn't changed.

## Complete Workflow

See `.claude/skills/pr-review/REVIEW_WORKFLOW.md` for the step-by-step process including:
- Checking for existing reviews (duplicate prevention)
- Generating inline comments
- Posting reviews
- Adding follow-up comments

## Review Criteria & Style

- Review criteria: `.claude/agents/pr-reviewer.md`
- Feedback style: `.claude/skills/pr-review/docs/tone.md`

## Resources

### Analysis & Prevention
- `.claude/skills/pr-review/REVIEW_WORKFLOW.md` - Complete workflow with duplicate prevention
- `.claude/skills/pr-review/scripts/analysis/analyze-review-status.sh` - Check if code changed since last review
- `.claude/skills/pr-review/scripts/posting/add-review-comment.sh` - Add follow-up comments instead of duplicate reviews
- `.claude/skills/pr-review/scripts/analysis/fetch-linked-issues.sh` - Fetch linked issues for requirements context
- `.claude/skills/pr-review/scripts/fetching/fetch-file.sh` - Cached file fetching (eliminates redundant API calls)
- `.claude/skills/pr-review/scripts/fetching/batch-fetch-files.sh` - Batch fetch multiple files with parallel processing
- `.claude/skills/pr-review/scripts/processing/chunk-diff.sh` - Split large diffs into token-limited chunks

### Code Generation & Posting
- `.claude/skills/pr-review/scripts/generation/generate-comment.sh` - Generate single inline comment (validates inputs)
- `.claude/skills/pr-review/scripts/generation/build-comments.sh` - Batch generate comments from text format
- `.claude/skills/pr-review/scripts/posting/post-review.sh` - Submit review with inline comments to GitHub

### Reference & Context
- `.claude/skills/pr-review/docs/github-inline-comments.md` - Inline comment API reference
- `.claude/skills/pr-review/docs/diff-workflow.md` - Token-efficient diff-based review
- `.claude/skills/pr-review/docs/ci-reference.md` - What CI validates (skip commenting on CI-caught issues)
- `.claude/skills/pr-review/scripts/fetching/filter-diff.sh` - Filters PR diff to exclude auto-generated files
- `.claude/skills/pr-review/templates/follow-up-issue.md` - Follow-up issue template
