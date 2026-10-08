---
name: herdr-sync-upstream
description: Sync the herdr fork with upstream herdrdev/herdr — weekly merge of upstream/master on a sync/<date> branch in its own worktree, lost-feature audit, fast-forward adopt, inventory update, disable new upstream bot workflows. Use when the user wants to pull/merge/integrate/sync upstream changes into the fork.
---

# Syncing the herdr fork with upstream

Fork model (since the v0.9.3 sync, #171): `master` is **merge-based**. It holds the
fork's own history, one merge commit per upstream sync, and fix commits. It is not
a linear patch set on top of upstream. Do not rebase `master` and do not
force-push it. A sync is `git merge upstream/master` on a scratch branch, and
adopting it is a fast-forward of `master`.

Cadence: weekly (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/173).
A small weekly merge is cheap; a skipped month is how v0.9.3 became a TUI port.

Topic order lives in `FORK-INVENTORY.md`, not in the history. Read it first:
it says what the fork carries, what is a port, what upstream now covers
(superseded) and what is a deliberate divergence. New authored commits carry
trailers (`Fork-Topic`, `Fork-Change`, `Upstream`, see the inventory).

Remotes: `origin` = github.com/colangelo/herdr-max, `internal` = Gitea
(AC-forks/herdr-max), `upstream` = github.com/herdrdev/herdr (`gh`/API calls use
`herdrdev/herdr` too). The fork's CI and `release-ac` pass
`--repo colangelo/herdr-max` explicitly, so upstream's default repo in
`scripts/changelog.py` is harmless.

Sync issue: open (or reuse) a `[fork] sync upstream <version/date>` issue on the
Gitea tracker before starting (`herdr-fork-tracking` skill). Everything found
below is recorded there.

## 1. Assess

```bash
git fetch upstream
BASE=$(git merge-base master upstream/master)                         # upstream tip of the last sync
git log master..upstream/master --oneline | wc -l                    # upstream commits since last sync
git log "$BASE"..upstream/master --oneline | head -80              # same range, readable
git diff "$BASE" upstream/master -- AGENTS.md                      # read it all
git log master..upstream/master --oneline -- .github/workflows justfile scripts/ build.rs .gitignore
git diff --stat "$BASE" upstream/master -- src/ | tail -40         # where upstream churned
```

(`BASE` is the upstream commit the last sync merged; no message search needed.)

Read these on purpose:

- **Upstream `AGENTS.md` diff**, especially "Stable client endpoint contract".
  It says which socket methods and snapshot fields are frozen. A change there
  decides how the fork's client-facing shape changes are allowed.
- **Architecture moves.** Look at renamed or deleted paths:
  `git diff -M --name-status "$BASE" upstream/master | grep -E '^(R|D)' | head -60`.
  The v0.9.3 lesson: upstream `207be3c7` moved the whole TUI into
  `src/client/shell/`. A move like that turns every fork hunk in the old files
  into a port, not a conflict. When you see one, stop and tell the coordinator
  before merging: it needs a plan and likely seats, not a quick resolve.
- **Protocol number.** `src/protocol/wire.rs::PROTOCOL_VERSION` upstream vs fork.
- **Integration asset versions** (`*_INTEGRATION_VERSION`) on both sides.

Where fork code lives now: code that only draws or handles input belongs in
`src/client/shell/`. Server facts reach the shell through optional snapshot
fields or advertised methods, never through private client sockets. A new fork
feature that needs server data adds an optional field (default keeps old
clients working) or a new advertised method.

## 2. Merge on a sync branch, in its own worktree

Never merge in the shared checkout. Use `wt`:

```bash
wt switch --create sync/$(date +%F) --base master
git -c merge.conflictStyle=diff3 merge upstream/master
```

Tag the revert point first. It is the old `master`:

```bash
git tag -a revert-point/pre-<upstream-version> master -m "fork master before the <version> sync"
git push origin revert-point/pre-<upstream-version>
git push internal revert-point/pre-<upstream-version>
```

diff3 shows the common ancestor, which you need to tell "upstream rewrote it"
from "fork added next to it". Keep `rerere` off for the first sync after an
architecture move: bad recordings poison later merges (`-c rerere.enabled=false`).

A `#![allow(dead_code, unused_imports)]` at the top of `src/main.rs` is
acceptable **temporarily** while ports land, so the base compiles and passes
clippy. It must be removed before the push, wiring or deleting each item
clippy then reports.

## 3. Lessons from v0.9.3 (each with the fix)

**A whole-tree merge beats a commit-by-commit replay for producing the tree.**
The replay's auto-resolver spliced code. Merge once, fix the result, and do not
re-derive the fork's commits one by one. Use the old commits as *spec* for
ports, not as patches.

**Fork cherry-picks of upstream commits come back as duplicates.** Symptoms:
`E0428`, `E0592`, `E0124`, `E0201`, `E0062` (duplicate definitions, impls,
fields, methods), plus duplicate tests, match arms and JSON entries that
compile fine. Fix: keep upstream's copy unless the fork's is a real extension
(then extend upstream's, once). Never both. Search the changed files for
repeated `fn test_name`, repeated match patterns and repeated JSON keys.

**Taking upstream's side of a region can silently drop fork code next to it,
and git's own "clean" merges can splice code.** A green build proves nothing.
Do both of these before calling the merge done:

1. Run the full suite (`cargo nextest run --locked --no-fail-fast`, see §5).
2. Do a **lost-feature audit.** For each fork commit that touched a file
   upstream rewrote (`git log --oneline "$BASE"..master -- <file>` for each
   file in `git diff --name-only "$BASE" upstream/master`), check that the
   behaviour still exists: grep for the function, read the call site, run the
   test. Do not trust "no conflict". The v0.9.3 audit found four losses that
   compiled and passed: the client reconnect after live handoff (#94), the
   re-exec onto a newer server build (#165), the hint-driven blocked ring and
   toasts (`a9c099df`), and the 200 ms sync-frame cap (`839473d8`, issue 126).
   Write the findings on the sync issue with full Gitea URLs, one line each:
   commit, what was lost, where it was restored (or why it was dropped).

**Contract fixtures.** Never re-bless generation-1 fixtures
(`tests/fixtures/endpoint-method-shapes-v1.json`). Fork shape changes (for
example the close methods' optional `force`) are frozen explicitly in
`advertised_client_shell_method_shapes_stay_at_the_v1_contract`. New fork
methods are frozen beside upstream's additive ones. A failing contract test is
a signal to change the fork side, not the fixture.

**The protocol number collides every sync.** Both sides bump from the same base.
Expect to bump the fork one past upstream's. Sweep for the number, not the
commit: numeric pins (`assert_eq!(value["result"]["protocol"], N)`) and string
pins (`contains("  protocol: N")`, Linux-only CLI suite). Keep
`tests/cli/sessions.rs` equal to `src/protocol/wire.rs::PROTOCOL_VERSION`.

**Regenerate the API schema** after any socket/method/shape change:

```bash
HERDR_UPDATE_API_SCHEMA=1 just test-one generated_protocol_schema_artifact_is_current
```

**Integration assets.** When both sides bumped an integration version with
different content (claude hook v10 in v0.9.3), bump once more so installs
refresh, and update the version assertions in `src/integration/tests.rs`.
Check the merged asset contains both sides' content, not just one.

**Infra** (`justfile`, `.github/workflows/`, `scripts/`, `build.rs`): do a
3-way `git merge-file` per file onto upstream's new layout
(`git merge-file -p ours base theirs`; base = the file at `$LAST^2`,
theirs = upstream's new file). Re-apply the fork deltas, do not fight the diff:
the `ZIG` export, the `release-ac`/beta flow, the macOS `windows-lint` skip,
and the fork manifest at `distribution/latest.json` plus the
`website/latest.json` compat copy for old 0.8.x-ac binaries. Checklist in §5.

**Handoff manifest compatibility.** The fork keeps its handoff manifest layout
(`agent_state` string plus `hook_agent_state`) so older `-ac` servers hand off
into the new build (`handoff-compat` in the inventory). Keep it when upstream
touches those fields.

**Disk on m4m.** Several worktrees times (debug + test + clippy) fill the disk
fast. Serialize `just check` runs, one worktree at a time, and keep the target
dirs (a cold rebuild costs more than the space). Check `df -h /` before starting.

**Seats in sandboxes.** Codex seats in `workspace-write` sandboxes cannot write
`.git` or bind Unix sockets. Use the committer and the NEEDS-RUN runner:

- committer: the seat writes `.local/port/ready/NNN.files` (paths) and
  `NNN.msg` (commit message); the committer commits them.
- runner: the seat writes a NEEDS-RUN request for an allowlisted test command;
  results land in `.local/port/runs/RESULTS.md`.
- scripts: `port-notes/tools/port-committer.sh` and `port-runner.sh` on branch
  `research/post-v0.9.3-port-notes`.
- **Never approve a seat's request to run outside the sandbox.** Route it to
  the coordinator.

**Changelog duplicates.** Both sides edit `## Unreleased` in
`docs/next/CHANGELOG.md`. Merge per `###` heading (upstream entries first),
one block per heading, in `Added` / `Changed` / `Fixed` order. A naive merger
folds Unreleased entries into a released section, or leaves two `### Fixed`
blocks. `scripts/changelog.py` copies the Unreleased body verbatim into the next
release, so duplicates ship. `just check` does not catch it. Check:

```bash
grep '^## ' docs/next/CHANGELOG.md | sort | uniq -d     # must be empty
awk '/^## /{s=$0; split("",h); split("",e); next} /^### /{if (h[$0]++) print "dup heading in " s ": " $0; next} /^- /{if (e[$0]++) print "dup entry in " s ": " substr($0,1,50)}' docs/next/CHANGELOG.md
```

**Other repeat conflicts.** `.gitignore`: keep both (upstream appends where the
fork `.env` block sits). `justfile` `test`/`check`: both sides add
`scripts.test_*` modules to the same `unittest` line, take the union. Manifests:
fork `claude.toml` versions can run ahead of upstream's; check
`distribution/agent-detection`, `release-docs-check`, and that the remote
catalog URL cannot override the fork's bundled manifests. Vendored
`libghostty-vt`: for each entry in `vendor/libghostty-vt.patches.md`, re-apply or
drop, and let `just check` verify.

## 4. Live-handoff proof before cutover

A merge that passes tests can still break live handoff. Prove it on a throwaway
session, never on a working one (memory recipe: "herdr throwaway proof recipe"):

1. Start a throwaway server on the *old* beta (sized tmux, `unset HERDR_ENV`,
   compiled stand-in `claude`, `SHELL=/opt/homebrew/bin/zsh`).
2. Create workspaces, tabs, panes, an agent pane, todos, pins, labels.
3. `herdr-beta server live-handoff --import-exe <new build>` (use the
   release-path build: a debug build answers on the `herdr-dev` socket and an old
   client hangs "reconnecting", which is an artifact, not a bug).
4. Compare before and after: workspaces/tabs/panes, pane pids and start times,
   sizes (with a client attached), agent state/session id/restore argv, pins,
   todos, labels. A new client must render and take input; an attached old
   client must reconnect (#94).
5. Agent screens stay blank until the agent repaints. The post-handoff nudge
   (SIGWINCH) causes the repaint; a stand-in that never repaints stays blank,
   that is the same as before the sync.

Record the result and evidence paths on the sync issue.

## 5. Verify before adopting

Fork surface survived:

```bash
git diff upstream/master HEAD --stat
git diff upstream/master HEAD --diff-filter=D --name-only      # no upstream file deleted without a reason
```

Load-bearing infra hunks:

- `release.yml`: `update-homebrew` job (publishes `Formula/herdr.rb` to
  colangelo/homebrew-tap); build env `HERDR_BUILD_CHANNEL: ac` plus the
  `Set fork build id from tag` step; tag-verify compares
  `TAG_BASE="${TAG_VERSION%%-ac*}"` against `Cargo.toml`; upstream's
  `update-latest-json` job (needs `secrets.RELEASE_DEPLOY_KEY`) stays removed,
  and the fork's own `update-latest-json` (uses `--repo colangelo/herdr-max`
  with `--tag`/`--force`) stays; it writes `distribution/latest.json` and the
  `website/latest.json` compat copy.
- `ci.yml`: the conventional-commits force-push guard
  (`git cat-file -e "$BEFORE_SHA"` fallback).
- `justfile`: `release-ac` recipe, `ZIG` export, macOS `windows-lint` skip.

```bash
uv run --with pyyaml python -c "import yaml; [yaml.safe_load(open(f)) for f in ['.github/workflows/release.yml','.github/workflows/ci.yml']]"
just --list >/dev/null
python3 scripts/conventional_commits.py --range "master..HEAD"   # new commits only
```

`master..HEAD` includes upstream's commits. A rejected upstream subject is not
yours to reword.

Full suite:

- `just check` (serialized, see disk note). Its test stage runs
  `cargo nextest run --locked --no-fail-fast`. Plain nextest cancels on the first
  failure and hides the rest, so a one-off looks like a broad breakage.
- **Remove the `#![allow(dead_code, unused_imports)]`** from `src/main.rs`
  first, and make clippy pass without it.
- Changelog checks from §3.
- `just release-docs-check` separately before any release (`just check` omits it).

**macOS `just check` does not compile the CLI integration suite.**
`tests/cli.rs` starts with `#![cfg(all(unix, not(target_os = "macos")))]`, so a
changed shared helper compiles clean locally and fails Linux CI with `E0061`.
Compile it before pushing:

```bash
cp tests/cli.rs /tmp/cli.rs.orig
sed -i.bak 's/^#!\[cfg(all(unix, not(target_os = "macos")))\]$/#![cfg(unix)]/' tests/cli.rs
rm -f tests/cli.rs.bak
cargo check --locked --all-targets
cp /tmp/cli.rs.orig tests/cli.rs                 # ALWAYS restore
git diff --name-only tests/cli.rs                # must be empty
```

Compile only, do not run. Never cross-compile with `cargo check --target
*-linux-gnu`: it clobbers the vendored libghostty-vt archive.

**Before blaming the merge for a failing test, get an upstream baseline:**

```bash
wt switch --create upstream-baseline --base upstream/master
cargo nextest run --locked <test_name>           # in that worktree
wt remove upstream-baseline
```

Known upstream-baseline failures on macOS, re-verify each sync, do not assume
they are still upstream's (all reproduced on clean upstream, zero fork patches,
deterministic under `-j 1`; last verified 2026-08-26 at `d79fd746`):

- `api_ping::events_subscribe_streams_output_and_agent_status_events`
- `cross_area::cross_area_two_clients_shared_view_and_single_detach_stability`
- `live_handoff::live_server_holds_one_pty_master_fd_per_pane`
- `multi_client::multi_client_broadcasts_frame_updates_to_all_clients`
- `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` (2026-07-25)

The fork's external-contributor guardrail forbids opening an upstream issue for
them. Compare against the baseline count, not against zero.

## 6. Adopt and push

Only when `just check` passes **and the coordinator says go**.

```bash
# on master in the shared checkout (or via wt merge): fast-forward only
git merge --ff-only sync/<yyyy-mm-dd>
git push origin master
git push internal master
```

No force, no lease. If `--ff-only` refuses, `master` moved: merge `master` into
the sync branch, rerun `just check`, try again.

If the internal push times out: Tailscale may be stopped (`tailscale status`;
`tailscale up`), or see the m4m MagicDNS caveat in global CLAUDE.md.

Then:

1. **Update `FORK-INVENTORY.md`.** Move every topic upstream now covers to
   **Superseded** and name the upstream commit (`upstream 7f89b11a (fork
   73a00623)`). Add new divergences. Note the new merge commit's parents.
2. **Comment on the sync issue** with full Gitea URLs (never bare `#N`, which
   means GitHub here): merge SHA, upstream range, the lost-feature audit list,
   handoff-proof result, test counts versus baseline, revert tag. Close the
   issue only after the dogfood below.
3. Leave the `revert-point/pre-<version>` tag in place until the next sync lands.

## 7. Post-sync checks

- **New upstream bot workflows.** Upstream's maintainer automation needs their
  secrets (`KANGAL_GITHUB_TOKEN`, `RELEASE_DEPLOY_KEY`) and fails on the fork.
  Compare `gh workflow list --repo colangelo/herdr-max --all` against the
  disabled set (Approve Contributor, Approve Merged Contributor, Issue Gate,
  Close pending-release issues, PR Gate, Preview, **Website**) and
  `gh workflow disable <name>` any new ones. Keep: CI, Nix, Release,
  Build artifacts (manual).
- **CI on the pushed master** goes green:
  `gh run list --repo colangelo/herdr-max --branch master --limit 3`.
- **Update manifest.** Run `just latest-json-check`. New fork binaries read
  `distribution/latest.json`; 0.8.x-ac binaries read the `website/latest.json`
  copy. Until the first `release-ac` after the sync runs, `distribution/latest.json`
  holds upstream's data, so cut the cutover release soon or an update offers
  upstream assets. Every target needs a 64-char `sha256`. `latest-json-check`
  also validates the copy published at raw.githubusercontent, so it stays red
  until the push lands and the raw cache turns over. Drop `website/latest.json`
  once no machine runs 0.8.x-ac.
- **Drift check.** Skim upstream changes to `justfile` release recipes,
  `scripts/changelog.py` and `release.yml`. If the release flow moved, update
  `.claude/skills/herdr-release/SKILL.md` and the `release-ac` recipe.
- **Versioned release docs stay upstream-only.** `docs-versions.mjs check`
  asserts the docs manifest matches `website/latest.json`, which the fork keeps
  fork-scoped, so it always fails here. `release-docs-check` omits that line and
  the `Website` workflow is disabled. If a sync reintroduces either, drop it
  again. Rationale: `.claude/skills/herdr-release/SKILL.md`.
- **Dogfood with `herdr-beta`.** Cut a beta (`herdr-dogfood` / `herdr-release`
  skills) and live-handoff the named Macs onto it, panes and sessions kept
  (`herdr-beta server live-handoff`). Report the before/after diff. Only then
  close the sync issue.
- Keep the fork's PROJECTS entry honest:
  `~/_sync/dev/CONTEXT/PROJECTS/herdr.md`.

Related: `FORK-INVENTORY.md`, `.claude/skills/herdr-release/SKILL.md` (the -ac
release after a sync), `.claude/skills/herdr-dogfood/SKILL.md`,
`~/_sync/dev/CONTEXT/SKILLS/fork-maintenance/SKILL.md` (the general pattern).
