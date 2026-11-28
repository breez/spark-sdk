#!/usr/bin/env python3
"""Post a breez-review result as one GitHub PR review with inline comments.

Reads review.md (the summary) and findings.json (line-anchored findings). A
finding is posted inline only when its lines are in the PR diff, because GitHub
rejects comments elsewhere. The rest are listed in the summary with a link.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request


def diff_lines(base):
    """Map each changed file to the right-side line numbers GitHub can comment on."""
    out = subprocess.run(
        ["git", "diff", "--unified=3", f"{base}...HEAD"],
        check=True, capture_output=True, text=True,
    ).stdout
    lines, path, new_line = {}, None, 0
    for row in out.splitlines():
        if row.startswith("+++ "):
            path = None if row == "+++ /dev/null" else row[6:]
            lines.setdefault(path, set())
        elif row.startswith("@@"):
            new_line = int(re.match(r"@@ -\S+ \+(\d+)", row).group(1))
        elif path is None or row.startswith(("--- ", "diff ", "index ")):
            continue
        elif row.startswith("+") or row.startswith(" "):
            lines[path].add(new_line)
            new_line += 1
    return lines


def comment_body(f):
    body = f"**{f['severity']}: {f['title']}**\n\n{f['body'].strip()}"
    return body


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--pr", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--head", required=True)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    summary = open("review.md").read().rstrip()
    findings = json.load(open("findings.json")) if os.path.exists("findings.json") else []
    valid = diff_lines(a.base)

    inline, outside = [], []
    for f in findings:
        path, line = f.get("path"), f.get("line")
        start = f.get("start_line")
        ok = path in valid and line in valid[path] and (start is None or start in valid[path])
        if ok:
            c = {"path": path, "line": line, "side": "RIGHT", "body": comment_body(f)}
            if start is not None and start < line:
                c.update(start_line=start, start_side="RIGHT")
            inline.append(c)
        else:
            outside.append(f)

    general = [f for f in outside if not f.get("path")]
    outside = [f for f in outside if f.get("path")]
    if general:
        summary += "\n\n### General findings\n\n" + "".join(
            f"- **{f['severity']}: {f['title']}** {f['body'].strip()}\n" for f in general)
    if outside:
        summary += "\n\n### Findings outside the diff\n\n"
        for f in outside:
            where = f.get("path") or "general"
            if f.get("path") and f.get("line"):
                url = f"https://github.com/{a.repo}/blob/{a.head}/{f['path']}#L{f['line']}"
                where = f"[`{f['path']}:{f['line']}`]({url})"
            summary += f"- **{f['severity']}: {f['title']}** ({where}). {f['body'].strip()}\n"

    payload = {"commit_id": a.head, "event": "COMMENT", "body": summary, "comments": inline}
    print(f"{len(inline)} inline, {len(outside)} outside the diff, {len(general)} general", file=sys.stderr)
    if a.dry_run:
        print(json.dumps(payload, indent=1))
        return

    req = urllib.request.Request(
        f"https://api.github.com/repos/{a.repo}/pulls/{a.pr}/reviews",
        data=json.dumps(payload).encode(), method="POST",
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}",
                 "Accept": "application/vnd.github+json"},
    )
    try:
        urllib.request.urlopen(req)
    except urllib.error.HTTPError as e:
        # A rejected anchor fails the whole review; fall back to the summary alone so
        # the PR still gets the review, and list the inline findings in it.
        print(f"Review with inline comments failed ({e.code}): {e.read().decode()}", file=sys.stderr)
        extra = "\n\n### Findings\n\n" + "\n".join(
            f"- `{c['path']}:{c['line']}` {c['body'].splitlines()[0]}" for c in inline)
        subprocess.run(["gh", "pr", "comment", a.pr, "--repo", a.repo,
                        "--body", summary + extra], check=True)


if __name__ == "__main__":
    main()
