# Diff-Based Review Workflow

Review the diff with context instead of reading full files to improve token efficiency and line number accuracy.

## Fetching PR Diff

```bash
# Default: filtered diff with standard context (3 lines)
.claude/skills/pr-review/scripts/fetching/filter-diff.sh $PR_NUMBER

# More context for complex changes (trait impls, error handling)
gh pr diff $PR_NUMBER --unified=20 -- path/to/file.rs

# Specific file only
gh pr diff $PR_NUMBER -- path/to/file.rs
```

## When to Read Full Files

Only read full files when the diff context is insufficient:

| Scenario | Why Full File Needed |
|----------|---------------------|
| Trait implementations | Need to see all required methods |
| Module structure | Understanding imports and exports |
| Removed code verification | Check if removed code is referenced elsewhere |
| Cross-function impact | Verify callers/callees of changed code |

## Size Check Before Full Reads

Check file size before reading to avoid exceeding token limits (~25,000 tokens):

```bash
# Estimate tokens (4 chars ≈ 1 token)
HEAD=$(gh pr view $PR_NUMBER --json headRefName -q .headRefName)
size=$(gh exec git show "$HEAD:path/to/file.rs" 2>/dev/null | wc -c)
tokens=$((size / 4))

if [ $tokens -gt 25000 ]; then
  echo "File likely exceeds token limit (estimated ~$tokens tokens)"
  # Use diff with extended context instead
  gh pr diff $PR_NUMBER --unified=50 -- path/to/file.rs
fi
```

For files exceeding the limit, rely on diff context or use targeted reads for specific line ranges.

## Line Range Extraction

To ensure comments display properly in GitHub's UI, always use multi-line ranges with 2-4 lines of context.

### Process

1. **Identify**: Find the issue in the diff
2. **Get context**: Use `gh api` to fetch 2-4 lines before and after the problematic line
3. **Extract range**: Record start_line and end line (inclusive) from the actual file

### Example

```bash
# For an issue on line 857 in sqlite.rs, get lines 855-860 for context
gh api "repos/breez/spark-sdk/contents/crates/breez-sdk/core/src/persist/sqlite.rs?ref=COMMIT_SHA" \
  | jq -r '.content' | base64 -d | awk 'NR>=855 && NR<=860 {printf "%4d  %s\n", NR, $0}'
```

Then create a multi-line comment:
```json
{
  "path": "crates/breez-sdk/core/src/persist/sqlite.rs",
  "start_line": 856,
  "line": 859,
  "side": "RIGHT",
  "start_side": "RIGHT",
  "body": "**Blocking** - SQL injection on line 857..."
}
```

**Critical:** Always include `start_line` and `start_side` fields. Single-line comments may not display properly in large diff hunks.

See `.claude/skills/pr-review/docs/github-inline-comments.md` for the two-step posting workflow.
