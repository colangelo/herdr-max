#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Oracle for the work panel (fork issue 241, task 2.16): the issue rows a herdr
server serves equal what the forge says.

Panel side: `issue.list {state: all}` on a herdr API socket whose server reads
the live ISSUES bucket (a throwaway session with the kv backend, never the
running one). Forge side: every issue of the projects the forge's bot can read.

    scripts/oracle_issue_rows.py --socket <herdr.sock> gitea  [--out report.json]
    scripts/oracle_issue_rows.py --socket <herdr.sock> gitlab [--group 146]

The bot token is read once from OpenBao (`kv/infra/worklink/gitea-bot`,
`kv/infra/worklink/gitlab-bot`) and kept in memory; it is never printed or
written. The forges are only read.

The contract (worklink's mirror): ref (forge, forge_id, host, project_id,
project_path, number), title, state, labels and assignees as sets, author,
created_at and updated_at as instants (the mirror stores UTC with a `Z`),
web_url. A title cut at 500 runes (`truncated`) is a prefix of the forge title.
`source_rev` and `last_sync` are not compared. GitLab calls an open issue
`opened`; the row says `open`.

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
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Callable, Iterator

TITLE_RUNES = 500


# -- the panel side -----------------------------------------------------------------


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


def panel_rows(sock_path: str) -> tuple[list[dict], list[dict], dict]:
    """Every issue the server lists, with its projects and mirror state."""
    issues: list[dict] = []
    while True:
        result = call(
            sock_path,
            "issue.list",
            {"state": "all", "limit": 2000, "offset": len(issues)},
        )["result"]
        issues.extend(result["issues"])
        if not result["issues"] or len(issues) >= result["total"]:
            return issues, result["projects"], result["mirror"]


# -- the forge side -----------------------------------------------------------------


def bao_field(path: str, field: str) -> str:
    out = subprocess.run(
        ["bao", "kv", "get", f"-field={field}", path],
        check=True,
        capture_output=True,
        text=True,
    )
    return out.stdout.strip()


def http_json(url: str, headers: dict[str, str]) -> list | dict:
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=60) as response:
        return json.load(response)


def paged(url: str, headers: dict[str, str], query: dict, size: int, page_key: str, size_key: str) -> Iterator[dict]:
    page = 1
    while True:
        qs = urllib.parse.urlencode({**query, size_key: size, page_key: page})
        batch = http_json(f"{url}?{qs}", headers)
        if isinstance(batch, dict):  # Gitea's repos/search wraps its list in "data"
            batch = batch.get("data", [])
        yield from batch
        if len(batch) < size:
            return
        page += 1


@dataclass
class Forge:
    """What differs between the forges: where the rows come from, and how a
    forge issue maps onto the contract's fields."""

    name: str  # the `ref.forge` of its rows
    projects: Callable[[], list[dict]]  # project dicts: id, path
    issues: Callable[[dict], Iterator[dict]]  # a project's issues, no pull requests
    fields: Callable[[dict], dict]  # forge issue -> contract fields


def gitea(base: str, owner: str) -> Forge:
    headers = {"Authorization": f"token {bao_field('kv/infra/worklink/gitea-bot', 'token')}"}
    api = f"{base}/api/v1"

    def projects() -> list[dict]:
        return [
            {"id": repo["id"], "path": repo["full_name"]}
            for repo in paged(f"{api}/repos/search", headers, {}, 50, "page", "limit")
            if repo["owner"]["login"] == owner and not repo.get("archived")
        ]

    def issues(project: dict) -> Iterator[dict]:
        for item in paged(
            f"{api}/repos/{project['path']}/issues", headers, {"state": "all", "type": "issues"}, 50, "page", "limit"
        ):
            if not item.get("pull_request"):
                yield item

    def fields(item: dict) -> dict:
        assignees = {a["login"] for a in item.get("assignees") or []}
        if item.get("assignee"):
            assignees.add(item["assignee"]["login"])
        return {
            "number": item["number"],
            "title": item["title"],
            "state": item["state"],
            "labels": {label["name"] for label in item.get("labels") or []},
            "assignees": assignees,
            "author": (item.get("user") or {}).get("login"),
            "created_at": item.get("created_at"),
            "updated_at": item.get("updated_at"),
            "web_url": item["html_url"],
        }

    return Forge("gitea", projects, issues, fields)


def gitlab(group: int) -> Forge:
    base = bao_field("kv/infra/worklink/gitlab-bot", "url").rstrip("/")
    headers = {"PRIVATE-TOKEN": bao_field("kv/infra/worklink/gitlab-bot", "token")}
    api = f"{base}/api/v4"

    def projects() -> list[dict]:
        return [
            {"id": project["id"], "path": project["path_with_namespace"]}
            for project in paged(
                f"{api}/groups/{group}/projects", headers, {"include_subgroups": "true"}, 100, "page", "per_page"
            )
            if not project.get("archived")
        ]

    def issues(project: dict) -> Iterator[dict]:
        # `issue_type=issue` keeps incidents and tasks out only if the mirror does;
        # the default (all work items GitLab calls issues) is what the mirror reads.
        yield from paged(f"{api}/projects/{project['id']}/issues", headers, {"state": "all", "scope": "all"}, 100, "page", "per_page")

    def fields(item: dict) -> dict:
        return {
            "number": item["iid"],
            "title": item["title"],
            "state": "open" if item["state"] == "opened" else item["state"],
            "labels": set(item.get("labels") or []),
            "assignees": {a["username"] for a in item.get("assignees") or []},
            "author": (item.get("author") or {}).get("username"),
            "created_at": item.get("created_at"),
            "updated_at": item.get("updated_at"),
            "web_url": item["web_url"],
        }

    return Forge("gitlab", projects, issues, fields)


# -- the comparison -----------------------------------------------------------------


def instant(stamp: str | None) -> datetime | None:
    if not stamp:
        return None
    return datetime.fromisoformat(stamp.replace("Z", "+00:00")).astimezone(timezone.utc)


def compare(row: dict, project: dict, forge: dict) -> list[str]:
    """Why the panel row differs from the forge issue (empty: equal)."""
    diffs: list[str] = []
    ref = row["ref"]
    for name, mine, theirs in (
        ("project_id", ref["project_id"], project["id"]),
        ("project_path", ref["project_path"], project["path"]),
        ("number", ref["number"], forge["number"]),
        ("state", row["state"], forge["state"]),
        ("author", row.get("author"), forge["author"]),
        ("web_url", row.get("web_url"), forge["web_url"]),
    ):
        if mine != theirs:
            diffs.append(f"{name}: panel {mine!r}, forge {theirs!r}")
    if row.get("truncated"):
        if (
            len(forge["title"]) <= TITLE_RUNES
            or len(row["title"]) != TITLE_RUNES
            or not forge["title"].startswith(row["title"])
        ):
            diffs.append(f"title: cut row is not the first {TITLE_RUNES} runes of the forge title")
    elif row["title"] != forge["title"]:
        diffs.append(f"title: panel {row['title']!r}, forge {forge['title']!r}")
    if set(row["labels"]) != forge["labels"]:
        diffs.append(f"labels: panel {sorted(row['labels'])}, forge {sorted(forge['labels'])}")
    if set(row["assignees"]) != forge["assignees"]:
        diffs.append(f"assignees: panel {sorted(row['assignees'])}, forge {sorted(forge['assignees'])}")
    for name in ("created_at", "updated_at"):
        if instant(row.get(name)) != instant(forge.get(name)):
            diffs.append(f"{name}: panel {row.get(name)!r}, forge {forge.get(name)!r}")
    return diffs


def main() -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    parser.add_argument("--socket", required=True, help="the herdr API socket of the throwaway session")
    parser.add_argument("--out", help="write the full report here as JSON")
    sub = parser.add_subparsers(dest="forge", required=True)
    one = sub.add_parser("gitea")
    one.add_argument("--base", default="https://gitea.cat-bluegill.ts.net")
    one.add_argument("--owner", default="ac", help="only repos of this owner")
    two = sub.add_parser("gitlab")
    two.add_argument("--group", type=int, default=146, help="the group (uzi-sandbox) whose projects are mirrored")
    args = parser.parse_args()

    issues, panel_projects, mirror = panel_rows(args.socket)
    if not mirror.get("loaded"):
        print(f"the panel server has not read the bucket: {mirror}", file=sys.stderr)
        return 2
    forge = gitea(args.base, args.owner) if args.forge == "gitea" else gitlab(args.group)

    # Rows of this forge only, keyed by (forge_id, project_id, number): ids of
    # different forges can collide.
    rows = {
        (r["ref"]["forge_id"], r["ref"]["project_id"], r["ref"]["number"]): r
        for r in issues
        if r["ref"]["forge"] == forge.name
    }
    syncs = {(p["forge_id"], p["project_id"]): p for p in panel_projects if p["forge"] == forge.name}
    forge_ids = {key[0] for key in rows} | {key[0] for key in syncs}
    if len(forge_ids) > 1:
        print(f"more than one {forge.name} instance in the bucket: {sorted(forge_ids)}", file=sys.stderr)
        return 2
    forge_id = next(iter(forge_ids), "")

    projects = forge.projects()
    problems: list[str] = []
    stale: list[str] = []
    equal = 0
    expected: dict[tuple, tuple[dict, dict]] = {}
    for project in projects:
        for item in forge.issues(project):
            fields = forge.fields(item)
            expected[(forge_id, project["id"], fields["number"])] = (project, fields)

    def stale_after_sync(key: tuple, fields: dict) -> bool:
        last_ok = instant((syncs.get((key[0], key[1])) or {}).get("last_ok"))
        touched = instant(fields.get("updated_at"))
        return bool(last_ok and touched and touched > last_ok)

    for key, (project, fields) in sorted(expected.items(), key=lambda kv: (kv[0][1], kv[0][2])):
        label = f"{project['path']}#{key[2]}"
        row = rows.get(key)
        if row is None:
            (stale if stale_after_sync(key, fields) else problems).append(f"{label}: not in the panel")
            continue
        diffs = compare(row, project, fields)
        if not diffs:
            equal += 1
        else:
            (stale if stale_after_sync(key, fields) else problems).append(f"{label}: {'; '.join(diffs)}")
    for key, row in sorted(rows.items(), key=lambda kv: (kv[0][1], kv[0][2])):
        if key not in expected:
            problems.append(f"{row['ref']['project_path']}#{key[2]}: in the panel, not on the forge")

    per_project: dict[tuple, int] = {}
    for key in rows:
        per_project[(key[0], key[1])] = per_project.get((key[0], key[1]), 0) + 1
    forge_projects = {(forge_id, p["id"]) for p in projects}
    for key in per_project:
        if key not in syncs or not syncs[key].get("synced"):
            problems.append(f"project {key[1]} has issue rows and no sync row")
    for key, project in syncs.items():
        if key not in forge_projects:
            problems.append(f"sync row for project {key[1]} ({project.get('project_path')}), not a project of the forge group")
        if project.get("issues_total") != per_project.get(key, 0):
            problems.append(f"project {key[1]}: issues_total {project.get('issues_total')} != rows {per_project.get(key, 0)}")
    for project in projects:
        if (forge_id, project["id"]) not in syncs:
            problems.append(f"{project['path']}: a project of the forge with no sync row")

    shown = [
        {
            "project": p.get("project_path"),
            "status": p.get("status"),
            "last_ok": p.get("last_ok"),
            "issues": p.get("issues_total"),
        }
        for p in sorted(syncs.values(), key=lambda p: p["project_id"])
    ]
    print(
        f"{forge.name}: panel {len(rows)} rows, forge {len(expected)} issues in {len(projects)} projects: "
        f"{equal} equal, {len(stale)} stale (expected), {len(problems)} mismatches"
    )
    for line in problems:
        print(f"MISMATCH {line}")
    for line in stale:
        print(f"stale    {line}")
    if args.forge == "gitlab":
        for line in shown:
            print(f"sync     {line['project']}: {line['status']}, last ok {line['last_ok']}, {line['issues']} issues")
    if args.out:
        with open(args.out, "w") as handle:
            json.dump(
                {
                    "forge": forge.name,
                    "panel_rows": len(rows),
                    "forge_rows": len(expected),
                    "projects": len(projects),
                    "equal": equal,
                    "stale_expected": stale,
                    "mismatches": problems,
                    "sync": shown,
                },
                handle,
                indent=2,
            )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
