---
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr checks:*), Bash(git diff:*), Bash(git log:*), Read, Grep, Glob
argument-hint: <pr-number>
description: Review a pull request against repository guidelines
---

## Step 1: Gather Context

Fetch PR details:
!`gh pr view $ARGUMENTS --json number,title,body,author,state,baseRefName,headRefName,additions,deletions,changedFiles 2>/dev/null || echo "Usage: /review_pr <pr-number>"`

## Step 2: Get the Diff

Choose strategy based on PR size from Step 1:

**Small PR** (<500 lines changed):
!`gh pr diff $ARGUMENTS`

**Medium PR** (500-2000 lines):
!`gh pr diff $ARGUMENTS --name-only`
Then fetch full diff for critical files (models, SDK interface, security-related), summarize the rest.

**Large PR** (>2000 lines):
!`gh pr diff $ARGUMENTS --name-only`
Review file-by-file, prioritizing:
1. API changes (`*/models.rs`, `*/sdk.rs`)
2. Security-sensitive code (`*/signer/*`, `*/crypto/*`)
3. Schema changes (`*/migrations/*`)
4. Tests last

Use `gh pr diff $ARGUMENTS -- <filepath>` for individual files.

## Step 3: Check CI Status

!`gh pr checks $ARGUMENTS 2>/dev/null || echo "CI status unavailable"`

## Step 4: Apply Review Criteria

- **Review criteria**: `.claude/rules/pr-review.md`
- **Technical reference** (build commands, binding files, architecture): `CLAUDE.md`

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