---
name: herdr-sync-upstream
description: Weekly sync of the herdr fork (Herdr Max) with upstream herdrdev/herdr. A merge of upstream/master on a sync branch, tags sync-merge/N, FORK-INVENTORY.md updates, checks only through the lead's runner under the shared lock, CI on a draft PR, then fast-forward master, a beta build and a seamless install. Use when the user or the weekly trigger says to sync, merge or pull in upstream changes.
---

# Weekly upstream sync of the herdr fork

Decided by ac on 2026-10-08 ("yes, weekly sync"):
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/173

## How the fork is laid out

- `master` is **merge-based**. It holds the fork's own history, one merge commit per
  upstream sync, and fix commits. Never rebase it, never force-push it, never replay the
  fork's commits one by one. The v0.9.3 sync (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/171)
  proved that a commit-by-commit replay splices code; a whole-tree merge does not.
- Cutover point: tag `cutover/0.9.3-ac` (master `ba033b4a`). Before it: tag `revert-point/pre-0.9.3`.
- Remotes: `origin` = github.com/colangelo/herdr-max (CI and releases run there),
  `internal` = Gitea AC-forks/herdr-max (issue tracker, integration remote, what you
  push to first), `upstream` = github.com/herdrdev/herdr (read only).
- Integration points are tags `sync-merge/N` (the counter is global; 30 at the cutover;
  next is `git tag -l 'sync-merge/*' | sort -V | tail -1` plus one). Tag every merge
  result, every green check and the commit you ask others to test.
- `FORK-INVENTORY.md` (repo root) is the map of what the fork carries. Rows have a status:
  **carried** (fork behaviour, present), **port** (re-implemented on the client shell),
  **divergent** (a deliberate difference from upstream), **superseded** (upstream does it
  now; fork code dropped). Read it before merging; update it after.
- New authored commits carry trailers `Fork-Topic`, `Fork-Change`, `Upstream` (see the
  inventory). Commit style: lowercase conventional commits, no emojis, no co-author or
  session trailer lines, and a body line `refs <full Gitea issue URL>` (a bare `#N` means
  GitHub here).

Where fork code lives: code that only draws or handles input belongs in
`src/client/shell/`. Server facts reach the shell through optional snapshot fields or
advertised methods, never private client sockets. Upstream's AGENTS.md "Stable client
endpoint contract" decides what may change on the wire.

## Roles and rules

- **Lead** = the session that owns the sync issue. For the weekly job the coordinator
  session `herdr` assigns it (or does it). The lead merges, fixes, runs checks and
  reports. Asks for ac go to the `herdr` coordinator, never to ac directly.
- **Heavy work goes through the lead's runner, under the shared lock.** The machine is
  shared by many sessions. No bare `cargo`, `just check`, `just test` or builds in your own
  shell. The runner is `port-notes/tools/port-runner.sh` on branch
  `research/post-v0.9.3-port-notes` (copy under `/tmp/herdr-sync/`). It takes
  `NEEDS-RUN: <command>` lines from `.local/port/PROGRESS.md` of a worktree, runs only an
  allowlist (`just check|test|lint|test-one <name>`, `cargo nextest run|test|check|clippy`),
  and writes `.local/port/runs/NNN.out` plus a line in `.local/port/runs/RESULTS.md`.
  What it runs, which you can also run by hand from the worktree:

  ```bash
  lockf -k /tmp/m4m-heavy.lock nice -n 15 env CARGO_BUILD_JOBS=6 CARGO_INCREMENTAL=0 \
    NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig timeout 2400 just check
  ```

  The lock is `/tmp/m4m-heavy.lock`; one heavy job at a time on the Mac. Cap jobs at 6
  (4 when the load is high). The runner only scans worktrees named `port-*`, `fix-*` and
  `sync-v0.9.3` under `~/dev/worktrees/herdr/`; for a new name, add it to its `for wt in`
  line or use the manual command above. Check `df -h /System/Volumes/Data` first; it stops
  at 97 %.
- **Docs-only syncs and the measurement run no cargo at all.**
- `herdr-beta`, never bare `herdr`. Worktrees through `wt`. `trash`, not `rm`. Never push
  to `master` before the coordinator says go. Gate every push of code on a passing
  `just check` exit code, chained: `... just check && git push ...`.

## 0. Measure (zero tokens, git only)

`scripts/sync_measure.sh` fetches `upstream` and `internal`, lists
`internal/master..upstream/master` with date, author, files and the files in
fork-heavy areas, shows the signals that decide the plan (upstream edits to `AGENTS.md`,
`src/protocol/wire.rs`, `Cargo.toml`, `justfile`; renamed or deleted paths), and runs
`git merge-tree --write-tree` to see whether the merge is textually clean. No checkout, no
merge, no build.

```bash
scripts/sync_measure.sh                 # report on stdout
scripts/sync_measure.sh --notify        # what the weekly job runs: file + bell to `herdr`
```

The weekly trigger runs it (see "The weekly trigger"). Zero commits: nothing to do.

Read these by hand when the report flags them:

- **Upstream `AGENTS.md` diff.** `git diff $(git merge-base internal/master upstream/master) upstream/master -- AGENTS.md`
- **Architecture moves.** `git diff -M --name-status "$BASE" upstream/master | grep -E '^(R|D)'`.
  The v0.9.3 lesson: upstream `207be3c7` moved the whole TUI into `src/client/shell/`,
  which turned every fork hunk in the old files into a port, not a conflict. If you see a
  move like that, stop and tell `herdr` before merging: it needs a plan and likely
  seats, not a quick resolve.
- **Protocol number.** The fork stays on upstream's `src/protocol/wire.rs::PROTOCOL_VERSION`
  (inventory row `wire-protocol`). Bump only with upstream.
- **Integration asset versions** (`*_INTEGRATION_VERSION`) on both sides.

Typical week: a handful of fixes and perf commits, no moves. Merge in one go.
A big week (renames, `AGENTS.md` contract change, protocol, 50+ commits) is merged in
steps: merge an upstream commit in the middle (`git merge <sha>`), check, tag, continue.

## 1. Open the sync issue

One Gitea issue per sync, **before** touching anything (skill `herdr-fork-tracking`):

- Title: `[fork] weekly upstream sync <YYYY-MM-DD>: <n> commits`
- Body: the measurement table (subject, date, files; group fixes / features / refactors /
  CI-docs), the flagged files with conflict risk, and the plan (one go or steps, expected
  conflicts, size of the check). No labels.
- Everything found later (lost-feature audit, test counts, handoff proof, beta version,
  install result) is a comment on it, with full Gitea URLs.
- Close it only after the install below, with a comment on how it was checked.

```bash
TOK=$(printf 'protocol=https\nhost=gitea.cat-bluegill.ts.net\n\n' | git credential fill 2>/dev/null | sed -n 's/^password=//p')
B="https://gitea.cat-bluegill.ts.net/api/v1"
curl -s -H "Authorization: token $TOK" -H 'Content-Type: application/json' \
  -d "$(jq -n --arg t "$title" --arg b "$body" '{title:$t,body:$b}')" \
  "$B/repos/AC-forks/herdr-max/issues"        # slug must be AC-forks/herdr-max (the old slug 301s and drops the body)
```

Never print the token. Build JSON with `jq -n --arg`.

## 2. Merge on a sync branch, in its own worktree

The branch `sync/merge` of #171 is an ancestor of `master` now and another worktree
may hold it; a weekly sync uses a dated branch `sync/<YYYY-MM-DD>` and keeps the
`sync-merge/N` tag counter.

```bash
git fetch upstream && git fetch internal
wt switch --create sync/$(date +%F) --base internal/master      # worktree under ~/dev/worktrees/herdr/
git tag -a revert-point/pre-$(date +%F) internal/master -m "fork master before the $(date +%F) sync"
git push internal revert-point/pre-$(date +%F)
git -c merge.conflictStyle=diff3 -c rerere.enabled=false merge upstream/master
```

(To merge in steps, name a commit instead of `upstream/master`.)

`diff3` shows the common ancestor, which you need to tell "upstream rewrote it" from "fork
added next to it". Keep `rerere` off: bad recordings poison later merges. Resolve, commit,
then `git tag -a sync-merge/N -m "<what this is>"` and push the tag to `internal`.

A `#![allow(dead_code, unused_imports)]` at the top of `src/main.rs` is acceptable only
temporarily while a port lands. Remove it before the push.

## 3. Rules from the v0.9.3 sync

**A green build proves nothing about a merge.** A merge can drop fork code next to a region
where you took upstream's side, and git's "clean" merges can splice code (the tree
`git merge-tree` calls clean in the measurement is a first guess, not a verdict). Do both
before calling the merge done:

1. Run the full suite through the runner (section 5).
2. **Lost-feature audit.** For each file in `git diff --name-only "$BASE" upstream/master`,
   list the fork commits that touched it (`git log --oneline "$BASE"..internal/master -- <file>`)
   and check the behaviour still exists: grep for the function, read the call site, run the
   test. Where the fork changed lines within a few lines of an upstream hunk, read both
   versions side by side. The v0.9.3 audit found four losses that compiled and passed:
   client reconnect after live handoff, re-exec onto a newer server build, the
   hint-driven blocked ring and toasts, the 200 ms sync-frame cap. One line each on the
   sync issue: commit, what was lost, where it was restored (or why it was dropped).

**Fork cherry-picks of upstream commits come back as duplicates** (`E0428`, `E0592`,
`E0124`, `E0201`, `E0062`; duplicate tests, match arms, JSON keys that compile fine). Keep
upstream's copy unless the fork's is a real extension; then extend upstream's, once.

**Contract fixtures.** Never re-bless generation-1 fixtures
(`tests/fixtures/endpoint-method-shapes-v1.json`). Fork shape changes (the close methods'
optional `force`) are frozen explicitly in
`advertised_client_shell_method_shapes_stay_at_the_v1_contract`. A failing contract test
means change the fork side, not the fixture.

**Regenerate the API schema** after any socket/method/shape change:
`HERDR_UPDATE_API_SCHEMA=1 just test-one generated_protocol_schema_artifact_is_current`
(through the runner).

**Integration assets.** When both sides bumped an integration version with different
content, bump once more so installs refresh and update the assertions in
`src/integration/tests.rs`. Check the merged asset has both sides' content.

**Handoff manifest compatibility** (inventory `handoff-compat`): the fork keeps its
manifest layout (`agent_state` string plus `hook_agent_state`) so older `-ac` servers hand
off into the new build. Keep it when upstream touches those fields.

**Infra files** (`justfile`, `.github/workflows/`, `scripts/`, `build.rs`). The fork deltas
are merged now, so a normal merge applies. When one conflicts, keep these: the `ZIG`
export, the `release-ac`/beta flow, the macOS `windows-lint` skip, the
`update-homebrew` job, `HERDR_BUILD_CHANNEL: ac`, the fork's own `update-latest-json`
(with `--repo colangelo/herdr-max`; upstream's one needing `RELEASE_DEPLOY_KEY` stays
removed), and `distribution/latest.json` plus the `website/latest.json` copy for 0.8.x-ac
binaries.

**Changelog duplicates.** Both sides edit `## Unreleased` in `docs/next/CHANGELOG.md`.
Merge per `###` heading, upstream entries first. `scripts/changelog.py` copies Unreleased
verbatim into the next release, so duplicates ship, and `just check` does not catch them:

```bash
grep '^## ' docs/next/CHANGELOG.md | sort | uniq -d     # must be empty
awk '/^## /{s=$0; split("",h); split("",e); next} /^### /{if (h[$0]++) print "dup heading in " s ": " $0; next} /^- /{if (e[$0]++) print "dup entry in " s ": " substr($0,1,50)}' docs/next/CHANGELOG.md
```

**Other repeat conflicts.** `.gitignore`: keep both. `justfile` `test`/`check`: both sides
add `scripts.test_*` modules to one line; take the union. Agent-detection manifests: the
fork's `claude.toml` versions can run ahead of upstream's; check `distribution/agent-detection`
and that the remote catalog cannot override the fork's bundled manifests. Vendored
`libghostty-vt`: for each entry in `vendor/libghostty-vt.patches.md`, re-apply or drop.

## 4. Live-handoff proof (only when the merge touches handoff, server or protocol)

A merge that passes tests can still break live handoff. Prove it on a throwaway session,
never on a working one (memory recipe "herdr throwaway proof recipe"):

1. Start a throwaway server on the old beta (sized tmux, `unset HERDR_ENV`, compiled
   stand-in `claude`, `SHELL=/opt/homebrew/bin/zsh`). Make workspaces, tabs, panes, an
   agent pane, todos, pins, labels.
2. `herdr-beta server live-handoff --import-exe <new release-path build>` (a debug build
   answers on the `herdr-dev` socket and an old client hangs "reconnecting", an artifact).
3. Compare before and after: workspaces/tabs/panes, pane pids and start times, sizes,
   agent state/session id/restore argv, pins, todos, labels. A new client must render and
   take input; an attached old client must reconnect.
4. Agent screens stay blank until the agent repaints; the post-handoff SIGWINCH nudge
   causes the repaint.

A build for this proof is heavy: ask for it through the runner and the lock.

## 5. Check, CI on a draft PR

Local check, through the runner (section "Roles and rules"):

- `just check` (its test stage runs `cargo nextest run --locked --no-fail-fast`; plain
  nextest cancels on the first failure and hides the rest).
- Changelog checks from section 3. `just release-docs-check` before any release.
- **macOS `just check` does not compile the CLI integration suite**
  (`tests/cli.rs` is `cfg(all(unix, not(target_os = "macos")))`), so a changed shared helper
  compiles locally and fails Linux CI with `E0061`. Compile it before pushing, then restore:

  ```bash
  cp tests/cli.rs /tmp/cli.rs.orig
  sed -i.bak 's/^#!\[cfg(all(unix, not(target_os = "macos")))\]$/#![cfg(unix)]/' tests/cli.rs && rm -f tests/cli.rs.bak
  cargo check --locked --all-targets       # through the runner; compile only, do not run
  cp /tmp/cli.rs.orig tests/cli.rs; git diff --name-only tests/cli.rs    # must be empty
  ```

  Never `cargo check --target *-linux-gnu`: it clobbers the vendored libghostty-vt archive.
- **Before blaming the merge for a failing test, get an upstream baseline** (a worktree at
  `upstream/master`, one test, through the runner). Known upstream failures change from
  sync to sync: re-verify, do not trust an old list. The fork cannot open upstream issues
  (external-contributor guardrail); record failures on the sync issue. Last known:
  https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/186 (an upstream profile
  test that fails on both builds).

Then push the branch and let CI judge on the full matrix (Linux, macOS, Windows):

```bash
git push internal sync/<date>
git push origin sync/<date>        # CI runs on GitHub
command gh pr create --repo colangelo/herdr-max --draft --base master --head sync/<date> \
  --title "weekly upstream sync <date>" --body "refs https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/<n>"
command gh pr checks --repo colangelo/herdr-max --watch
```

If the push to `origin` is refused (the GitHub token was revoked in October 2026), do not
look for another path: tell `herdr`, it asks ac. The draft PR is for CI only; nobody reviews
or merges it on GitHub.

Windows lint is skipped on macOS by design; the Windows CI job is where upstream's
Windows-only files (`windows_recent_fallback.rs` and similar) get compiled.

## 6. Adopt

Only when the check and the PR's CI are green **and `herdr` says go**:

```bash
cd <the shared checkout or the master worktree>
git fetch internal
git merge --ff-only sync/<date>      # no force, no lease
git push internal master
git push origin master
```

If `--ff-only` refuses, `master` moved: merge `master` into the sync branch, recheck, retry.
If the push to `internal` times out: `tailscale status`, then the m4m MagicDNS note in
global CLAUDE.md.

Then:

1. **Update `FORK-INVENTORY.md`**: move every topic upstream now covers to **superseded**
   and name the upstream commit; add new divergences; note the new merge commit. A
   pure-fix week often changes nothing; say so on the issue.
2. **Comment on the sync issue**: merge SHA, upstream range, lost-feature audit list,
   handoff proof result if run, test counts vs baseline, tags (`sync-merge/N`,
   `revert-point/pre-<date>`).
3. Leave the previous `revert-point/*` tag in place until the next sync lands, then keep
   or trash it by `herdr`'s call.

## 7. Beta and seamless install

The lead hands the master SHA to `herdr` (the coordinator pane), which builds the beta.
Mechanics: skills `herdr-release` (Beta channel, live handoff) and `herdr-dogfood`.

1. Beta build from pushed master (`just beta`), wait for the run, note
   `herdr-beta --version` (`X.Y.Z-ac-beta.<run>-<codename>`).
2. **Seamless install = live handoff**, panes and sessions kept
   (`herdr-beta server live-handoff`, or `just brew-upgrade herdr-beta`), **mac-m4
   first**. Report the before/after diff (workspaces, panes, pids, agents).
3. Then **mbm5**, through manager-aruba (herdr only asks; it does not touch that Mac).
4. Close the sync issue with the beta version, the two install results and the diff.

## 8. Post-sync checks

- **New upstream bot workflows.** Upstream's maintainer automation needs their secrets and
  fails on the fork. Compare `command gh workflow list --repo colangelo/herdr-max --all`
  against the disabled set (Approve Contributor, Approve Merged Contributor, Issue Gate,
  Close pending-release issues, PR Gate, Preview, **Website**) and
  `command gh workflow disable <name>` any new one. Keep CI, Nix, Release, Build artifacts
  (manual).
- **CI on the pushed master** is green: `command gh run list --repo colangelo/herdr-max --branch master --limit 3`.
- **Update manifest**: `just latest-json-check` (through the runner). New fork binaries read
  `distribution/latest.json`; 0.8.x-ac binaries read the `website/latest.json` copy. Until
  the first `release-ac` after a sync, `distribution/latest.json` may hold upstream's data.
  Every target needs a 64-char `sha256`. Drop `website/latest.json` once no machine runs 0.8.x-ac.
- **Drift check.** If upstream moved `justfile` release recipes, `scripts/changelog.py` or
  `release.yml`, update `.claude/skills/herdr-release/SKILL.md` and the `release-ac` recipe.
- **Versioned release docs stay upstream-only**: `docs-versions.mjs check` asserts the docs
  manifest matches `website/latest.json`, which the fork keeps fork-scoped, so it always
  fails here. `release-docs-check` omits that line and the Website workflow is disabled.
- Keep the fork's PROJECTS entry honest: `~/_sync/dev/CONTEXT/PROJECTS/herdr.md`.

## The weekly trigger

What runs: **a zero-token measurement script, not a session wake.** Nothing needs a model
until there is something to merge. `scripts/sync_measure.sh --notify` fetches, measures,
writes `~/.local/state/herdr-sync/<date>.md`, and when upstream has new commits rings the
`herdr` coordinator with `agent-bell send --to herdr --wake` (the fallback is `--queue`, so
the report is not lost; the peer-message rule is in CONTEXT `PATTERNS/agent-bell.md`). With
zero commits it stays silent. The coordinator then opens the sync issue, assigns a lead (or
leads it) and runs sections 1 to 8.

Why not the `scheduled-checks` skill: that is a session-only `CronCreate` job (dies with
the session, expires after 7 days). #173 asks for a durable trigger. A LaunchAgent is.

Where: **m4m**, which owns the Mac's scheduled jobs (like `dev.agent-idle-maintain`).
When: **Tuesday 08:43** local time. Not Sunday 10:00 (budget reset), not :00 or :30; a
Tuesday leaves the week's budget fresh after the Sunday reset and Wednesday to Friday to
dogfood. A Mac asleep at 08:43 runs the job on wake.

What mac-m4 installs (nobody has installed it yet): `~/Library/LaunchAgents/dev.herdr.sync-measure.plist`

```xml
<dict>
  <key>Label</key><string>dev.herdr.sync-measure</string>
  <key>ProgramArguments</key><array>
    <string>/opt/homebrew/bin/gtimeout</string><string>10m</string>
    <string>/bin/zsh</string><string>-lc</string>
    <string>cd ~ &amp;&amp; HERDR_REPO=$HOME/_sync/dev/herdr exec $HOME/_sync/dev/herdr/scripts/sync_measure.sh --notify</string>
  </array>
  <key>StartCalendarInterval</key><dict>
    <key>Weekday</key><integer>2</integer><key>Hour</key><integer>8</integer><key>Minute</key><integer>43</integer>
  </dict>
  <key>RunAtLoad</key><false/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>/Users/ac/Library/Logs/herdr-sync-measure.log</string>
  <key>StandardErrorPath</key><string>/Users/ac/Library/Logs/herdr-sync-measure.err</string>
</dict>
```

(`zsh -lc` puts `agent-bell` and Homebrew on PATH.) Before relying on it: the script path
must exist on the shared checkout, i.e. this file has to be on `master` there; test once with
`launchctl kickstart -k gui/$UID/dev.herdr.sync-measure` and confirm `herdr` gets the bell.
`herdr` also keeps one standing line on its weekly list: "Tuesday sync report arrived?
if not, run `scripts/sync_measure.sh` by hand".

Related: `FORK-INVENTORY.md`, `.claude/skills/herdr-release/SKILL.md`,
`.claude/skills/herdr-dogfood/SKILL.md`, `.claude/skills/herdr-fork-tracking` (the global
skill), `~/_sync/dev/CONTEXT/SKILLS/fork-maintenance/SKILL.md` (the general pattern),
`~/_sync/dev/CONTEXT/SKILLS/scheduled-checks/SKILL.md`.
