---
name: herdr-sync-upstream
description: Sync the herdr fork with upstream herdrdev/herdr — scratch-branch rebase of the fork patches onto upstream/master, force-push to origin and internal, disable new upstream bot workflows. Use when the user wants to pull/merge/integrate/sync upstream changes into the fork.
---

# Syncing the herdr fork with upstream

Fork model: `master` = `upstream/master` + a linear patch set. Never merge —
rebase the patch set onto the new upstream tip and force-push. History SHAs
change on every sync; that is expected.

The patch set is no longer small: it was ~150 commits as of the 2026-07-25
sync, mostly sidebar/UI features, and it touches `src/` broadly (`src/app/`,
`src/api/`, `src/ui/`, `src/cli/`, `src/config/`). Budget for real source
conflicts, not just workflow-file ones.

Remotes: `origin` = github.com/colangelo/herdr-max, `internal` = Gitea
(ac/herdr), `upstream` = github.com/herdrdev/herdr. Upstream moved out of
`ogulcancelik/herdr` into the `herdrdev` org (2026-07); the `upstream` remote
URL was repointed at the new slug on 2026-08-26, so it no longer rides the
GitHub redirect, and `gh`/API calls must use `herdrdev/herdr` as well.
Upstream's `scripts/changelog.py` `DEFAULT_RELEASE_REPO` says `herdrdev/herdr`
(harmless here: the fork's CI and `release-ac` pass `--repo colangelo/herdr-max`
explicitly).

## 1. Assess

```bash
git fetch upstream
git log upstream/master..master --oneline        # the fork patch set
git merge-base master upstream/master            # old upstream base
git rev-list --left-right --count upstream/master...master
git log master..upstream/master --oneline -- .github/workflows justfile scripts/changelog.py .gitignore
```

The last command previews conflict risk on the fork's infrastructure files:
`.github/workflows/release.yml`, `.github/workflows/ci.yml`, `justfile`,
`.gitignore`, `.claude/skills/`, `.codex/skills/`, `.pi/skills/`. Also diff
the fork-touched source files against upstream's churn:

```bash
git diff --name-only $(git merge-base master upstream/master) master | grep '^src/'
git log master..upstream/master --oneline -- src/
```

## 2. Rebase on a scratch branch

Tag the pre-sync tip first — it is the revert point, and §5's lease refers to it:

```bash
git tag -a revert-point/$(date +%F)-pre-upstream-sync master -m "known-good fork master before the sync"
git push origin --follow-tags && git push internal --follow-tags
```

```bash
BASE=$(git merge-base master upstream/master)
git branch -f reconcile-test master
git checkout reconcile-test
git rebase --onto upstream/master "$BASE" reconcile-test
```

**Force `rerere.autoupdate` off for the replay.** It is on globally here, and
during a rebase it stages its own replayed resolution — which leaves
`rebase --continue` reporting *"you have staged changes in your working tree"*
and refusing to proceed, with no `stopped-sha`/`message` left in
`.git/rebase-merge` to explain why. Run every continue as
`git -c rerere.autoupdate=false rebase --continue`. If you hit the state anyway,
recover by committing the staged resolution under the stopped commit's own
identity — `git commit --no-edit -C $(awk '/^pick /{s=$2} END{print s}' \
"$(git rev-parse --git-dir)/rebase-merge/done")` — then continue. (Hit
2026-08-10; cost ~20 min of misdiagnosis.)

**A "keep both sides" auto-resolver must check each side is brace-balanced.**
Unions are right for import lists, struct fields and match arms that both sides
merely *append* to. They are wrong when the two sides share a trailing
continuation — upstream's last match arm and the fork's new arm both ending in
one `leave_navigate_mode(); }` — because concatenating splices one arm into the
other. They are also wrong when the sides are *alternative implementations* of
the same block (upstream reimplementing a feature the fork already has); those
are brace-balanced, so balance alone will not catch them and the tell is a pile
of `unused variable` warnings over a duplicated preamble. Log every auto-merged
block and re-read the log after `cargo check`.

Known conflict patterns:
- `src/app/state.rs` and the overlay call sites (`src/app/input/{mod,modal,mouse,
  navigate,settings,overlays}.rs`, `src/ui/{navigator,settings,keybind_help,menus}.rs`)
  — the fork's overlay kit (Gitea #65) replaced `Mode` plus ten-plus parallel
  `Option<XState>` fields on `AppState` with one `Option<Overlay>` and a set of
  generated accessors. `Mode` and `AppState` are what upstream edits most, so
  every upstream change that adds an overlay field or reads one conflicts here.
  Resolve by taking upstream's *behaviour* and expressing it through the kit:
  a new upstream overlay state becomes a line in the `overlays!` list rather
  than a new field, and `state.foo.bar` becomes `state.foo().bar` or one of the
  `set_foo_*` helpers. `AppState::assert_invariants_for_test` catches a
  resolution that leaves mode and overlay disagreeing; the overlay snapshot
  tests catch one that moves an overlay on screen.
- `.gitignore` — upstream appends entries where the fork `.env` block sits: keep both.
- `.github/workflows/release.yml` — upstream restructures jobs; re-apply the fork
  hunks (see §4 checklist) rather than fighting the diff.
- `docs/next/CHANGELOG.md` — conflicts on nearly every fork docs commit, because
  both sides edit `## Unreleased`. Resolve by merging per `###` heading (upstream
  entries first, then fork entries), keeping ONE block per heading in the
  `Added` / `Changed` / `Fixed` order the released sections already use.
  **Automate this carefully** — two distinct ways it has gone wrong:
  1. A naive merger keying only on `###` merges across a `## [X.Y.Z]` release
     boundary and folds upstream's Unreleased entries into a released section.
  2. A resolution that keeps *both* sides' blocks leaves two `### Fixed` blocks
     in one section with the first entries duplicated between them. Hit on the
     2026-07-25 sync and only caught 2026-07-26, by eye: `## Unreleased` had
     `Added / Fixed / Changed / Fixed`, with two entries appearing twice.

  This is not cosmetic. `SECTION_RE` in `scripts/changelog.py` matches only
  `##`, so `prepare_release` copies the Unreleased body **verbatim** into the new
  version section — duplicate headings and repeated entries ship in the real
  release notes. `just check` does not catch it (`scripts.test_changelog` tests
  the tooling, not the document).

  After the rebase, verify BOTH:

  ```bash
  # 1. no duplicate ## version headings; sections reverse-chronological
  grep '^## ' docs/next/CHANGELOG.md | sort | uniq -d     # must be empty
  # 2. no repeated ### block or entry inside any one section — silence is pass
  awk '/^## /{s=$0; split("",h); split("",e); next} /^### /{if (h[$0]++) print "dup heading in " s ": " $0; next} /^- /{if (e[$0]++) print "dup entry in " s ": " substr($0,1,50)}' docs/next/CHANGELOG.md
  ```

  (One line on purpose — verified on macOS stock awk 20200816. A heredoc here
  breaks on paste, because its closing delimiter ends up indented.)

  When repairing an already-duplicated section, rebuild it programmatically with
  assertions on every line index rather than editing by eye, then prove nothing
  was lost:
  `diff <(grep '^- ' OLD | sort -u) <(grep '^- ' NEW | sort -u)` must be empty.
- `justfile` `test`/`check` recipes — both sides add `scripts.test_*` modules to
  the same `python3 -m unittest` line: take the union.
- `tests/cli/sessions.rs` — asserts a hardcoded protocol number. Keep the fork's
  value (fork protocol is intentionally ahead of upstream's) and make sure it
  matches `src/protocol/wire.rs::PROTOCOL_VERSION`.
- **The protocol number collides every sync.** Both sides bump independently from
  the same base, so upstream keeps landing on the number the fork already took
  (19 → both, 20 → both). Expect to bump the fork one past upstream's every time.
  Git may DROP the fork's previous bump commits as redundant when upstream's
  value catches up — that is correct, but it means the tree silently reverts to
  upstream's number and you must re-bump. **Sweep for the number, not the
  commit**, and note the pins come in two shapes: numeric
  (`assert_eq!(value["result"]["protocol"], N)`) and string
  (`stdout.contains("  protocol: N")` — the latter only runs in the Linux CLI
  suite, so macOS `just check` will not catch it). Also regenerate the schema
  artifact: `HERDR_UPDATE_API_SCHEMA=1 cargo nextest run
  generated_protocol_schema_artifact_is_current`.
- If a rebase goes sideways, `git rebase --abort` and retry; master is untouched
  until §5.

## 3. Never rebase orphaned release commits

Fork release commits (`release: vX.Y.Z-ac`, doc promotions) become orphaned
after each sync — do NOT carry them forward. Only the durable patch set
rebases. Release tags stay valid pointing at orphaned commits.

## 4. Verify the fork surface survived

```bash
git diff upstream/master reconcile-test --stat
```

Expect the fork's feature surface plus its infrastructure files; what must NOT
appear is deletions of upstream files
(`git diff upstream/master reconcile-test --diff-filter=D --name-only` should
be empty). Then check the load-bearing hunks in
`.github/workflows/release.yml`:

- `update-homebrew` job present (publishes Formula/herdr.rb to colangelo/homebrew-tap)
- build job env has `HERDR_BUILD_CHANNEL: ac` (+ the `Set fork build id from tag` step)
- tag-verify compares `TAG_BASE="${TAG_VERSION%%-ac*}"` against Cargo.toml
- upstream's `update-latest-json` job stays REMOVED (it needs upstream deploy
  keys). Note the fork has its OWN `update-latest-json` job — one job with that
  name is correct; tell them apart by `secrets.RELEASE_DEPLOY_KEY` (upstream's)
  vs `--repo colangelo/herdr-max` + `--tag`/`--force` (the fork's).

And in `.github/workflows/ci.yml`: the conventional-commits force-push guard
(`git cat-file -e "$BEFORE_SHA"` fallback). And in `justfile`: the `release-ac`
recipe at the end. Then:

```bash
uv run --with pyyaml python -c "import yaml; [yaml.safe_load(open(f)) for f in ['.github/workflows/release.yml','.github/workflows/ci.yml']]"
just --list >/dev/null
```

Then run `just check`. Use `cargo nextest run --locked --no-fail-fast` for the
test stage — nextest cancels the run on first failure by default, so a single
failure hides the other ~1600 tests and you cannot tell a one-off from a broad
breakage.

`just check` does NOT validate the changelog document, so run the two
`docs/next/CHANGELOG.md` duplicate checks from §2 here as well — a bad merge
resolution there is silent until it reaches published release notes. Note also
that `just check` omits `release-docs-check`; run that separately before any
release.

**Before blaming the rebase for a test failure, get an upstream baseline.**
Build clean `upstream/master` in a scratch worktree and run the same test
there:

```bash
wt switch --create upstream-baseline --base upstream/master
cargo nextest run --locked <test_name>          # in that worktree
wt remove upstream-baseline                     # when done
```

Known upstream-baseline failures (macOS). Each was reproduced on a clean
`upstream/master` worktree with zero fork patches applied, so none is a rebase
regression — but re-verify against the baseline each sync rather than assuming
they are still upstream's.

- Verified 2026-07-25 at upstream `d4e0dd3d`:
  `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` fails with
  `agent_not_found`.
- Verified 2026-08-26 at upstream `d79fd746` (v0.8.2+), all three deterministic
  under `-j 1`, i.e. not load flakes:
  - `api_ping::events_subscribe_streams_output_and_agent_status_events` —
    timed out waiting for json line
  - `cross_area::cross_area_two_clients_shared_view_and_single_detach_stability`
    — `pane_read_recent_contains(... "SHARED_VIEW" ...)` never true
  - `live_handoff::live_server_holds_one_pty_master_fd_per_pane` —
    "had 2 ptmx master fds; expected 1"
  - `multi_client::multi_client_broadcasts_frame_updates_to_all_clients` —
    panic at `tests/multi_client.rs:955`

  Full fork `master` baseline that day: 3782 run, 3778 passed, these 4 failed.
  Compare a post-rebase run against that number, not against zero.

  All three are upstream-authored tests the fork last touched in April/May, and
  they fail the same way on fork `master`. They are an upstream/macOS problem,
  not fork breakage; the fork's external-contributor guardrail forbids opening
  an upstream issue for them.

**macOS `just check` does not compile the CLI integration suite at all.**
`tests/cli.rs` opens with `#![cfg(all(unix, not(target_os = "macos")))]`, so every
file under `tests/cli/` — the harness, the string protocol pins, ~20 case files —
is invisible to a local macOS build AND to `cargo check --all-targets`. A
resolution that changes a shared test helper's signature therefore compiles clean
on macOS and fails CI on Linux with a wall of `E0061`. Hit 2026-08-26: the fork's
liveness-bounds commit converted `wait_for_socket(path, timeout)` call sites to
one argument, the conflict resolution kept the two-argument definition in
`tests/cli/harness.rs`, and nothing local caught it.

Compile that suite before pushing, by lifting the exclusion in a throwaway edit:

```bash
cp tests/cli.rs /tmp/cli.rs.orig
sed -i.bak 's/^#!\[cfg(all(unix, not(target_os = "macos")))\]$/#![cfg(unix)]/' tests/cli.rs
rm -f tests/cli.rs.bak
cargo check --locked --all-targets            # now includes tests/cli/**
cp /tmp/cli.rs.orig tests/cli.rs              # ALWAYS restore
git diff --name-only tests/cli.rs             # must be empty
```

The tests themselves are excluded on macOS for runtime reasons, so do not try to
*run* them — compiling is the whole point. Do NOT cross-compile to Linux to check
this: `cargo check --target *-linux-gnu` clobbers the vendored libghostty-vt
archive in this checkout.

## 5. Adopt and push

```bash
git checkout master
OLD=$(git rev-parse master)
git reset --hard reconcile-test && git branch -D reconcile-test
git push --force-with-lease=master:$OLD origin master
git push --force-with-lease=master:$OLD internal master
```

If the internal push times out: Tailscale may be stopped (`tailscale status`;
`tailscale up`), or see the m4m MagicDNS caveat in global CLAUDE.md.

**CI's conventional-commits job validates the WHOLE patch set after a sync**, not
just the new commits: its range is `$BEFORE_SHA..$AFTER_SHA`, and the force-push
guard only narrows that to the head commit when `BEFORE_SHA` is *unreachable*.
Tagging the pre-sync tip (§2) keeps it reachable, so the job re-reads all ~230
subjects and any long-standing bad one fails the push. Check before pushing:

```bash
python3 scripts/conventional_commits.py --range "upstream/master..master"
```

Fix by rewording in place — a `rebase -i` over the patch set with
`GIT_SEQUENCE_EDITOR` marking the offenders `reword` and `GIT_EDITOR` rewriting
the subject. Prove it touched nothing else by comparing `git rev-parse
master^{tree}` before and after; the trees must be identical. (2026-08-10:
`style: rustfmt after rebase conflict resolutions` — `style` is not an allowed
type — and `ignore .vscode` had ridden along invisibly for months.)

## 6. Post-sync checks

- **New upstream bot workflows**: upstream's maintainer automation needs their
  secrets (`KANGAL_GITHUB_TOKEN`, `RELEASE_DEPLOY_KEY`) and fails on the fork.
  Compare `gh workflow list --repo colangelo/herdr-max --all` against the disabled
  set (Approve Contributor, Approve Merged Contributor, Issue Gate,
  Close pending-release issues, PR Gate, Preview, **Website**) and
  `gh workflow disable <name>` any new ones. Keep: CI, Nix, Release,
  Build artifacts (manual).
- **CI on the pushed master** must go green (`gh run list --repo colangelo/herdr-max
  --branch master --limit 3`). The conventional-commits job tolerates the
  force-push via the fork guard.
- **`website/latest.json` survived the rebase**: `just latest-json-check`.
  The release workflow's `update-latest-json` job commits the manifest to
  master *after* the tag, so it is an ordinary master commit that §5's
  force-push of a replayed patch set drops — silently, because the release
  itself already succeeded and no job goes red. `src/update.rs` reads that file
  over raw GitHub, so a dropped commit pins every fork binary's update check to
  an older version: exactly what happened to v0.7.4-ac, which went unnoticed
  from 2026-07-18 until 2026-07-27 (AC-forks/herdr-max#38). Restore it with the
  same call CI makes, using the tag's protocol version, not master's:

  ```bash
  TAG=v0.7.4-ac; BASE=${TAG#v}; BASE=${BASE%%-ac*}
  PROTOCOL=$(git show "$TAG:src/protocol/wire.rs" | sed -n 's/^pub const PROTOCOL_VERSION: u32 = \([0-9]*\);/\1/p')
  python3 scripts/changelog.py sync-latest-json --repo colangelo/herdr-max \
      --tag "$TAG" --version "$BASE" --protocol "$PROTOCOL" --force \
      --output website/latest.json
  ```

  Since upstream's *verify stable release checksums* (2026-08), that manifest
  must also carry a 64-char `sha256` per target, asserted by
  `update::tests::checked_in_website_manifest_matches_update_schema` and by
  `latest-json-check`. A manifest generated before that lands has no `sha256`
  map and fails both; regenerate it with the command above — GitHub already
  records asset digests for the older fork releases, so this needs no re-upload.
  `latest-json-check` also validates the manifest *published* at
  raw.githubusercontent, so it stays red until §5's push lands and the raw cache
  turns over.
- **Drift check**: skim upstream changes to `justfile` release recipes,
  `scripts/changelog.py`, and `release.yml` — if the release flow moved,
  update `.claude/skills/herdr-release/SKILL.md` and the `release-ac` recipe
  to match.
- **Versioned release docs stay upstream-only.** Upstream v0.7.5+ added
  `docs/versions/` + `website/scripts/docs-versions.mjs` + a `Website`
  workflow. `docs-versions.mjs check` asserts the docs manifest matches
  `website/latest.json`, which the fork keeps fork-scoped, so it always fails
  here. `release-docs-check` deliberately omits that line and the `Website`
  workflow is disabled — if a sync reintroduces either, drop it again rather
  than "fixing" the manifest. Full rationale: the "Versioned release docs are
  upstream-only" section of `.claude/skills/herdr-release/SKILL.md`.
- Keep the fork's PROJECTS entry honest:
  `~/_sync/dev/CONTEXT/PROJECTS/herdr.md`.

Related: `.claude/skills/herdr-release/SKILL.md` (cutting the -ac release
after a sync), `~/_sync/dev/CONTEXT/SKILLS/fork-maintenance/SKILL.md`
(the general pattern).
