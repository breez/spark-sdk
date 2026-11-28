# Report

## Human review needed

Score the change from 0 to 5 and map it to a level. Start at 0.

| Add | When |
|---|---|
| +1 | It moves money or touches keys, signing, deposits, claims, fees, or calls to operators or the SSP |
| +1 | It changes the public SDK API, bindings, a storage schema or a migration |
| +1 | It is large (over 1,000 changed lines outside lockfiles and generated code), or some changed files were not read |
| +1 | A Blocking finding remains, or a Should-fix finding sits on a money path |
| -1 | Only docs, tests, CI, comments, or a mechanical change (rename, dependency bump with no API change) |

Clamp to 0 to 5. The score is advisory: it guides how much review a change gets, and it does not replace the repo's approval rules.

| Score | Level | Means |
|---|---|---|
| 4 to 5 | Required | A careful review by someone who knows the area |
| 2 to 3 | Recommended | A normal review, focused on the listed areas |
| 0 to 1 | Optional | A quick skim |

Always name what a human should check: the design and API choices in the change.
Tools rarely judge those well.

## Format

Write the report in this order. Keep it short: the overview is 120 words at most,
each finding block 70 words at most, each one-line item 25 words at most.

```
## Review: <PR title>

**Human review (advisory): <Level> (<score>/5).** <One sentence on why.> Check: <the design or API choices and risk areas a human should look at>.

**In plain words:** <1 to 2 sentences.>
**How it works:** <3 to 5 one-line bullets.>
**Risk areas:** <2 to 5 one-line bullets.>

<sequence diagram, medium and large changes only>

**Blocking: N · Should-fix: N · Nits: N**

| # | Severity | Finding | Where |
|---|---|---|---|
| 1 | Blocking | <one-line title> | `file:line` |

### 1. <Severity>: <one-line title>
- **Where:** `file:line`
- **Problem:** <at most 2 sentences.>
- **Fix:** <at most 3 sentences, or a short code block headed by the file and lines.>

**More Should-fix:** one line each: the title, `file:line`, the fix.

**Nits:** the 5 most worth fixing, one line each.
```

- Give a full block to every Blocking finding and to the top 5 Should-fix findings.
  The remaining Should-fix findings go under "More Should-fix", one line each. They
  are still in the table.
- Remaining nits: locally, end with `N more nits. Ask to see them.` In CI, put them in a
  collapsed `<details><summary>N more nits</summary>...</details>` block.
- With no findings, say so in one line after the counts.

## CI output

In CI, write two files at the repo root instead of the format above.

`findings.json`: every finding tied to specific lines, Blocking, Should-fix and Nit alike, as a JSON array:

```json
[{"severity": "Should-fix", "title": "<one line>", "path": "crates/x/src/y.rs",
  "line": 120, "start_line": 112, "body": "<problem, at most 2 sentences>\n\n**Fix:** <at most 3 sentences or a short code block>"}]
```

- `path` is repo-relative. `line` is a line in the PR head version of the file, the
  last line of the range. `start_line` is optional, for a range of up to 15 lines.
- Point at the line that has the problem, in the changed code when there is one. Read
  the file to confirm the number; do not copy a number you have not checked.
- Only a finding about the change as a whole, with no line or group of lines to point
  at, stays out of this file.

`review.md`: the human review line, the overview, risk areas and diagram, the counts
line, a table of every finding (severity, title, `file:line`), and under
`### General findings` the findings that are not tied to lines, in full. No nit list:
nits are inline like everything else.
