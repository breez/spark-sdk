# Project Analytics

Pulls PR and commit data from GitHub + local git history and produces an Excel
workbook (`project_analytics.xlsx`) with team-velocity, review-time, and
code-churn analytics for the `breez/spark-sdk` repo.

## Output

`project_analytics.xlsx` contains these sheets:

| Sheet | Granularity | Key metrics |
|---|---|---|
| **Summary** | totals | date range, PR counts, merge rate, avg/median lead time, commits, lines added/deleted |
| **PRs** | one row per PR | title, author, state, created/merged/closed, lead time, base/head ref, +/−, files, commits, review count, first-review latency, labels, requested reviewers |
| **Weekly Velocity** | ISO week | PRs opened/merged/closed, commits, additions, deletions, unique PR/commit authors, cumulative merged |
| **Monthly Velocity** | YYYY-MM | same as weekly + avg/median lead time |
| **Author Stats** | per GitHub login | PRs opened/merged, merge rate, avg/median lead time, first/last PR |
| **Git Authors** | per commit-author name | commits to `main`, additions, deletions, files touched, first/last commit |
| **Commits** | one row per commit | sha, date, author, subject, additions, deletions, files, is_merge, week, month |

## Pipeline

```
fetch_pr_list.py  --->  data/prs_all.json
enrich_prs.py     --->  data/pr_enrichment.json   (optional, needs API calls)
git log --numstat --->  data/git_log_main.txt
                                                 build_analytics.py
                                                    --->  project_analytics.xlsx
```

## Refreshing the data

```bash
# Requires: openpyxl, requests. GITHUB_TOKEN env var with repo:read scope.

# 1. Pull the latest PR list (~7 pages for 600+ PRs; ~1s).
GITHUB_TOKEN=ghp_xxx python3 scripts/analytics/fetch_pr_list.py

# 2. Refresh the local git log.
git fetch --unshallow origin main 2>/dev/null || git fetch origin main
git log origin/main --numstat \
  --format='__COMMIT__%H%x09%aI%x09%an%x09%ae%x09%s' \
  > scripts/analytics/data/git_log_main.txt

# 3. (Optional) Enrich per-PR data for review times, +/− lines, etc.
#    This makes one API call per PR (~5-10 minutes for a full refresh,
#    incremental on subsequent runs).
GITHUB_TOKEN=ghp_xxx python3 scripts/analytics/enrich_prs.py

# 4. Build the spreadsheet.
python3 scripts/analytics/build_analytics.py
open scripts/analytics/project_analytics.xlsx
```

## Notes and caveats

- **Per-PR code changes (`additions`, `deletions`, `commits`, `first_review_hours`, etc.)
  are blank until you run `enrich_prs.py`.** The initial build uses the PR list
  endpoint, which does not include these fields — getting them requires one API
  call per PR. The Git Authors / Commits sheets and the totals in Summary use
  the local git log, which is always populated.
- **Commit `author` and PR `author` are not always the same identifier.** GitHub
  commits use the `user.name` from `.gitconfig`; PRs use the user's GitHub
  login. We show them separately (Author Stats vs Git Authors) rather than
  guessing a mapping.
- **Merge commits are excluded** from commit counts and line totals (they would
  double-count PR-merge noise).
- **Lead time** is `merged_at − created_at` in hours. Draft time and review
  wait are included; we don't try to subtract "developer idle" time.
- **ISO weeks** (e.g. `2026-W16`) are used so weekly buckets align with the ISO
  calendar rather than shifting relative to the start date.
