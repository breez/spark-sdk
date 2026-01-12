---
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr checks:*), Bash(git diff:*), Bash(git log:*), Read, Grep, Glob
description: Review a pull request against repository guidelines
---

# PR Review Command

You are reviewing PR #$ARGUMENTS for the Breez SDK repository.

## Step 1: Gather Context

Fetch PR details:
!`gh pr view $ARGUMENTS --json number,title,body,author,state,baseRefName,headRefName,additions,deletions,changedFiles 2>/dev/null || echo "Usage: /review_pr <pr-number>"`

## Step 2: Get the Diff

!`gh pr diff $ARGUMENTS 2>/dev/null | head -500`

(If diff is large, focus on the most critical files first)

## Step 3: Check CI Status

!`gh pr checks $ARGUMENTS 2>/dev/null || echo "CI status unavailable"`

## Step 4: Review Guidelines

Read and apply the review guidelines from `.claude/rules/pr-review.md`.

## Your Task

Provide a **compact, actionable review** structured as:

### Summary
1-2 sentences describing what this PR does.

### Review

**Code Quality**
- Formatting/linting concerns
- Rust conventions (error handling, patterns)

**Testing**
- Test coverage assessment
- Missing test cases

**API Changes** (if applicable)
- Binding file updates needed
- Breaking change considerations

**Security** (if applicable)
- Key handling, input validation

### Issues
List specific issues by severity (CRITICAL > HIGH > MEDIUM > LOW).
Format: `file:line - description`

### Recommendation
APPROVE | REQUEST CHANGES | COMMENT

---

Keep the review focused and avoid over-explaining. Prioritize actionable feedback.
