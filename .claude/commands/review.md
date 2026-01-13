---
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr checks:*), Bash(git diff:*), Bash(git log:*), Read, Grep, Glob
description: Review the current PR against repository guidelines
---

# PR Review Command

You are reviewing the current pull request for the Breez SDK repository.

## Step 1: Gather Context

Fetch current PR details:
!`gh pr view --json number,title,body,author,state,baseRefName,headRefName,additions,deletions,changedFiles`

## Step 2: Get the Diff

!`gh pr diff | head -500`

(If diff is large, focus on the most critical files first)

## Step 3: Check CI Status

!`gh pr checks`

## Step 4: Review Guidelines

Read and apply the review guidelines from `.claude/rules/pr-review.md`.

## Your Task

Provide a **concise, actionable review**. Only include sections with meaningful findings.

### Summary
1-2 sentences: what the PR does and the problem it solves.

### Design Analysis (only if concerns)
- Rationale, approach, trade-offs, extensibility
- Skip if design is sound

### Issues (only if any)
List by severity. Omit empty levels. Format: `file:line - description`
- CRITICAL / HIGH / MEDIUM / LOW

### Questions (only if needed)
Clarifications needed from author.

### Recommendation
**APPROVE** | **REQUEST CHANGES** | **COMMENT**

---

**Keep it short.** If everything passes, a review can be as simple as:

```
### Summary
Adds X to support Y.

### Recommendation
**APPROVE** - CI passes, tests included, design is sound.
```
