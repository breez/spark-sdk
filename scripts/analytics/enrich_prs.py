#!/usr/bin/env python3
"""Fetch per-PR enrichment (additions/deletions/commits/reviews/labels) from the GitHub REST API.

The base analytics build (``build_analytics.py``) works from the PR *list* endpoint, which does
not include code-change or review data. This script fills that gap.

Requires:
  - ``GITHUB_TOKEN`` env var (a classic or fine-grained token with `repo:read` scope)

Writes:
  - ``scripts/analytics/data/pr_enrichment.json`` — map of PR number -> enrichment dict.

Re-running is incremental: already-enriched PRs are skipped unless ``--force`` is passed.

Usage:
  GITHUB_TOKEN=ghp_xxx python3 scripts/analytics/enrich_prs.py
  GITHUB_TOKEN=ghp_xxx python3 scripts/analytics/enrich_prs.py --only 800 801 803
  GITHUB_TOKEN=ghp_xxx python3 scripts/analytics/enrich_prs.py --since 2026-01-01
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

import requests

OWNER = "breez"
REPO = "spark-sdk"
API = f"https://api.github.com/repos/{OWNER}/{REPO}"

ROOT = Path(__file__).resolve().parent
DATA = ROOT / "data"
OUT = DATA / "pr_enrichment.json"


def headers() -> dict[str, str]:
    tok = os.environ.get("GITHUB_TOKEN")
    if not tok:
        sys.exit("error: set GITHUB_TOKEN to run enrichment")
    return {
        "Authorization": f"Bearer {tok}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }


def get(url: str, params: dict | None = None) -> dict | list:
    for attempt in range(5):
        r = requests.get(url, headers=headers(), params=params, timeout=30)
        if r.status_code == 200:
            return r.json()
        if r.status_code in (403, 429) and "rate limit" in r.text.lower():
            reset = int(r.headers.get("X-RateLimit-Reset", "0"))
            wait = max(reset - int(time.time()), 5) + 2
            print(f"  rate-limited; sleeping {wait}s...", file=sys.stderr)
            time.sleep(wait)
            continue
        if r.status_code >= 500:
            time.sleep(2 ** attempt)
            continue
        r.raise_for_status()
    raise RuntimeError(f"failed after retries: {url}")


def enrich_pr(number: int) -> dict:
    pr = get(f"{API}/pulls/{number}")
    reviews = get(f"{API}/pulls/{number}/reviews", params={"per_page": 100})
    issue = get(f"{API}/issues/{number}")  # labels / comment count live here
    first_review_at = None
    if isinstance(reviews, list) and reviews:
        dates = [r.get("submitted_at") for r in reviews if r.get("submitted_at")]
        if dates:
            first_review_at = min(dates)
    return {
        "additions": pr.get("additions"),
        "deletions": pr.get("deletions"),
        "changed_files": pr.get("changed_files"),
        "commits": pr.get("commits"),
        "review_count": len(reviews) if isinstance(reviews, list) else 0,
        "comment_count": pr.get("comments", 0) + pr.get("review_comments", 0),
        "first_review_at": first_review_at,
        "requested_reviewers": [u["login"] for u in (pr.get("requested_reviewers") or [])],
        "labels": [l["name"] for l in (issue.get("labels") or [])] if isinstance(issue, dict) else [],
        "merged_by": ((pr.get("merged_by") or {}) or {}).get("login"),
    }


def load_prs() -> list[dict]:
    with open(DATA / "prs_all.json") as f:
        return json.load(f)


def load_existing() -> dict:
    if not OUT.exists():
        return {}
    with open(OUT) as f:
        return json.load(f)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", type=int, nargs="*", help="only enrich these PR numbers")
    ap.add_argument("--since", help="enrich PRs created on/after YYYY-MM-DD")
    ap.add_argument("--limit", type=int, default=None, help="cap number of PRs enriched this run")
    ap.add_argument("--force", action="store_true", help="re-fetch even if already cached")
    args = ap.parse_args()

    prs = load_prs()
    cache = load_existing()

    targets = prs
    if args.only:
        want = set(args.only)
        targets = [p for p in prs if p["number"] in want]
    if args.since:
        targets = [p for p in targets if p.get("created_at", "") >= args.since]
    if not args.force:
        targets = [p for p in targets if str(p["number"]) not in cache]
    if args.limit:
        targets = targets[: args.limit]

    print(f"Enriching {len(targets)} PRs (cached: {len(cache)})")
    for i, pr in enumerate(targets, 1):
        n = pr["number"]
        try:
            cache[str(n)] = enrich_pr(n)
            if i % 10 == 0 or i == len(targets):
                with open(OUT, "w") as f:
                    json.dump(cache, f, indent=2)
                print(f"  [{i}/{len(targets)}] saved (last PR #{n})")
        except Exception as e:
            print(f"  PR #{n}: {e}", file=sys.stderr)

    with open(OUT, "w") as f:
        json.dump(cache, f, indent=2)
    print(f"Wrote {OUT} ({len(cache)} PRs)")


if __name__ == "__main__":
    main()
