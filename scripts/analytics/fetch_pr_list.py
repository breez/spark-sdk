#!/usr/bin/env python3
"""Fetch the full PR list (paginated) from GitHub and write ``data/prs_all.json``.

Complements ``build_analytics.py`` so the whole pipeline is self-refreshing:

    fetch_pr_list.py  -> data/prs_all.json          (state, author, dates)
    enrich_prs.py     -> data/pr_enrichment.json    (commits, +/- lines, reviews)
    build_analytics.py -> project_analytics.xlsx

Usage:
    GITHUB_TOKEN=ghp_xxx python3 scripts/analytics/fetch_pr_list.py
"""

from __future__ import annotations

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
OUT = DATA / "prs_all.json"


def headers() -> dict[str, str]:
    tok = os.environ.get("GITHUB_TOKEN")
    if not tok:
        sys.exit("error: set GITHUB_TOKEN")
    return {
        "Authorization": f"Bearer {tok}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }


def main() -> None:
    all_prs: list[dict] = []
    page = 1
    while True:
        r = requests.get(
            f"{API}/pulls",
            headers=headers(),
            params={"state": "all", "per_page": 100, "page": page,
                    "sort": "created", "direction": "asc"},
            timeout=30,
        )
        if r.status_code == 403 and "rate limit" in r.text.lower():
            reset = int(r.headers.get("X-RateLimit-Reset", "0"))
            wait = max(reset - int(time.time()), 5) + 2
            print(f"rate-limited; sleeping {wait}s", file=sys.stderr)
            time.sleep(wait)
            continue
        r.raise_for_status()
        batch = r.json()
        if not batch:
            break
        all_prs.extend(batch)
        print(f"page {page}: +{len(batch)} (total {len(all_prs)})")
        page += 1

    DATA.mkdir(parents=True, exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(all_prs, f, indent=2)
    print(f"Wrote {OUT} ({len(all_prs)} PRs)")


if __name__ == "__main__":
    main()
