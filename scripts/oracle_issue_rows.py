#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Oracle for the work panel (fork issue 241, task 2.16): the issue rows a herdr
server serves equal what the forge says.

Panel side: `issue.list {state: all}` on a herdr API socket whose server reads
the live ISSUES bucket (a throwaway session with the kv backend, never the
running one). Forge side: every issue of the repos the Gitea bot can read.

    scripts/oracle_issue_rows.py --socket <herdr.sock> [--out report.json]

The bot token is read once from OpenBao (`kv/infra/worklink/gitea-bot`) and kept
in memory; it is never printed or written. Gitea is read only.

The contract (worklink's mirror): ref (forge, forge_id, host, project_id,
project_path, number), title, state, labels and assignees as sets, author,
created_at and updated_at as instants (the mirror stores UTC with a `Z`),
web_url. A title cut at 500 runes (`truncated`) is a prefix of the forge title.
`source_rev` and `last_sync` are not compared.

The bucket is a snapshot, so a row whose forge `updated_at` is later than its
project's last good sync is "stale, expected", not a mismatch. Exit status 1
means a real mismatch.
"""

from __future__ import annotations

import argparse
import json
import socket
import subprocess
import sys
import urllib.parse
import urllib.request
from datetime import datetime, timezone

TITLE_RUNES = 500
PAGE = 50  # Gitea's own cap on a page


def panel_rows(sock_path: str) -> tuple[list[dict], list[dict], dict]:
    """Every issue the server lists, with its projects and mirror state."""
    issues: list[dict] = []
    offset = 0
    while True:
        answer = call(
            sock_path,
            "issue.list",
            {"state": "all", "limit": 2000, "offset": offset},
        )
        result = answer["result"]
        issues.extend(result["issues"])
        offset = len(issues)
        if not result["issues"] or offset >= result["total"]:
            return issues, result["projects"], result["mirror"]


def call(sock_path: str, method: str, params: dict) -> dict:
    with socket.socket(socket.AF_UNIX) as sock:
        sock.connect(sock_path)
        sock.sendall((json.dumps({"id": "oracle", "method": method, "params": params}) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = sock.recv(1 << 20)
            if not chunk:
                raise RuntimeError("the socket closed")
            data += chunk
    answer = json.loads(data)
    if "error" in answer:
        raise RuntimeError(f"{method}: {answer['error']}")
    return answer


def bot_token() -> str:
    out = subprocess.run(
        ["bao", "kv", "get", "-field=token", "kv/infra/worklink/gitea-bot"],
        check=True,
        capture_output=True,
        text=True,
    )
    return out.stdout.strip()


def get(base: str, token: str, path: str, query: dict) -> list | dict:
    url = f"{base}/api/v1{path}?{urllib.parse.urlencode(query)}"
    request = urllib.request.Request(url, headers={"Authorization": f"token {token}"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def pages(base: str, token: str, path: str, query: dict):
    page = 1
    while True:
        batch = get(base, token, path, {**query, "limit": PAGE, "page": page})
        if isinstance(batch, dict):  # repos/search wraps its list in "data"
            batch = batch.get("data", [])
        yield from batch
        if len(batch) < PAGE:
            return
        page += 1


def forge_rows(base: str, token: str, owner: str) -> tuple[dict[tuple[int, int], dict], list[dict]]:
    """Issues (not pull requests) of every unarchived repo of `owner`."""
    repos = [
        repo
        for repo in pages(base, token, "/repos/search", {})
        if repo["owner"]["login"] == owner and not repo.get("archived")
    ]
    rows: dict[tuple[int, int], dict] = {}
    for repo in repos:
        for item in pages(
            base,
            token,
            f"/repos/{repo['full_name']}/issues",
            {"state": "all", "type": "issues"},
        ):
            if item.get("pull_request"):
                continue
            rows[(repo["id"], item["number"])] = {**item, "_repo": repo}
    return rows, repos


def instant(stamp: str | None) -> datetime | None:
    if not stamp:
        return None
    return datetime.fromisoformat(stamp.replace("Z", "+00:00")).astimezone(timezone.utc)


def compare(row: dict, forge: dict) -> list[str]:
    """Why the panel row differs from the forge issue (empty: equal)."""
    diffs: list[str] = []
    repo = forge["_repo"]
    ref = row["ref"]
    for name, mine, theirs in (
        ("forge", ref["forge"], "gitea"),
        ("project_id", ref["project_id"], repo["id"]),
        ("project_path", ref["project_path"], repo["full_name"]),
        ("number", ref["number"], forge["number"]),
        ("state", row["state"], forge["state"]),
        ("author", row.get("author"), (forge.get("user") or {}).get("login")),
        ("web_url", row.get("web_url"), forge["html_url"]),
    ):
        if mine != theirs:
            diffs.append(f"{name}: panel {mine!r}, forge {theirs!r}")
    if row.get("truncated"):
        if len(forge["title"]) <= TITLE_RUNES or not forge["title"].startswith(row["title"]) or len(row["title"]) != TITLE_RUNES:
            diffs.append(f"title: cut row is not the first {TITLE_RUNES} runes of the forge title")
    elif row["title"] != forge["title"]:
        diffs.append(f"title: panel {row['title']!r}, forge {forge['title']!r}")
    labels = {label["name"] for label in forge.get("labels") or []}
    if set(row["labels"]) != labels:
        diffs.append(f"labels: panel {sorted(row['labels'])}, forge {sorted(labels)}")
    assignees = {a["login"] for a in forge.get("assignees") or []}
    if forge.get("assignee"):
        assignees.add(forge["assignee"]["login"])
    if set(row["assignees"]) != assignees:
        diffs.append(f"assignees: panel {sorted(row['assignees'])}, forge {sorted(assignees)}")
    for name in ("created_at", "updated_at"):
        if instant(row.get(name)) != instant(forge.get(name)):
            diffs.append(f"{name}: panel {row.get(name)!r}, forge {forge.get(name)!r}")
    return diffs


def main() -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    parser.add_argument("--socket", required=True, help="the herdr API socket of the throwaway session")
    parser.add_argument("--base", default="https://gitea.cat-bluegill.ts.net", help="the Gitea base URL")
    parser.add_argument("--owner", default="ac", help="only repos of this owner")
    parser.add_argument("--out", help="write the full report here as JSON")
    args = parser.parse_args()

    issues, projects, mirror = panel_rows(args.socket)
    if not mirror.get("loaded"):
        print(f"the panel server has not read the bucket: {mirror}", file=sys.stderr)
        return 2
    forge, repos = forge_rows(args.base, bot_token(), args.owner)

    sync = {p["project_id"]: p for p in projects if p.get("synced")}
    panel = {(r["ref"]["project_id"], r["ref"]["number"]): r for r in issues}
    report: dict = {"panel_rows": len(panel), "forge_rows": len(forge), "repos": len(repos)}
    problems: list[str] = []
    stale: list[str] = []
    equal = 0

    def stale_after_sync(project_id: int, forge_issue: dict) -> bool:
        last_ok = instant((sync.get(project_id) or {}).get("last_ok"))
        touched = instant(forge_issue.get("updated_at"))
        return bool(last_ok and touched and touched > last_ok)

    for key, forge_issue in sorted(forge.items()):
        label = f"{forge_issue['_repo']['full_name']}#{key[1]}"
        row = panel.get(key)
        if row is None:
            (stale if stale_after_sync(key[0], forge_issue) else problems).append(f"{label}: not in the panel")
            continue
        diffs = compare(row, forge_issue)
        if not diffs:
            equal += 1
        elif stale_after_sync(key[0], forge_issue):
            stale.append(f"{label}: {'; '.join(diffs)}")
        else:
            problems.append(f"{label}: {'; '.join(diffs)}")
    for key, row in sorted(panel.items()):
        if key not in forge:
            problems.append(f"{row['ref']['project_path']}#{key[1]}: in the panel, not on the forge")

    per_project: dict[int, int] = {}
    for project_id, _ in panel:
        per_project[project_id] = per_project.get(project_id, 0) + 1
    for project_id in per_project:
        if project_id not in sync:
            problems.append(f"project {project_id} has issue rows and no sync row")
    for project in projects:
        if project["project_id"] in per_project and project.get("issues_total") not in (None, per_project[project["project_id"]]):
            problems.append(f"project {project['project_id']}: issues_total {project['issues_total']} != rows {per_project[project['project_id']]}")

    report.update(equal=equal, stale_expected=stale, mismatches=problems)
    print(
        f"panel {len(panel)} rows, forge {len(forge)} issues in {len(repos)} repos: "
        f"{equal} equal, {len(stale)} stale (expected), {len(problems)} mismatches"
    )
    for line in problems:
        print(f"MISMATCH {line}")
    for line in stale:
        print(f"stale    {line}")
    if args.out:
        with open(args.out, "w") as handle:
            json.dump(report, handle, indent=2)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
