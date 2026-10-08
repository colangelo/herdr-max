---
name: herdr-release
description: Cut an -ac suffixed release of the herdr fork — promotes staged docs, runs just release-ac, publishes binaries to GitHub Releases and the colangelo/homebrew-tap formula. Also covers the rolling -ac-beta channel and upgrading/migrating a running install between binaries/channels via live handoff (stable<->beta, brew upgrades) without killing panes. Use when the user wants to release, ship, publish, tag, cut a new version, or upgrade/switch/migrate a herdr install between stable and beta.
---

# Releasing herdr (fork, -ac channel)

Fork versioning keeps the upstream base visible: release `X.Y.Z-ac` where
`X.Y.Z` is the upstream version the tree is based on (`grep '^version' Cargo.toml`).
Re-releases on the same base: `X.Y.Z-ac.2`, `X.Y.Z-ac.3`.

Why this shape: `Cargo.toml` MUST stay plain `X.Y.Z` — `src/update.rs
Version::parse` panics on suffixes. The `-ac` suffix reaches the binary via
`HERDR_BUILD_CHANNEL=ac` (+ `HERDR_BUILD_ID` for `.N`) set in the release
workflow, so `herdr --version` prints `X.Y.Z-ac` with zero Rust patches.
Upstream's own preview builds use the same mechanism.

## Beta channel (-ac-beta), for pre-release testing

A rolling, opt-in beta that ships from `master` **without** a version tag or a
stable release, distributed as a **separate** tap formula so it coexists with
stable herdr. Use it to dogfood work (e.g. a freshly synced/featured `master`)
before cutting the stable `X.Y.Z-ac`.

Trigger (dispatches `.github/workflows/beta.yml`; default ref `master`):

```bash
just beta            # build + publish beta from master
just beta my-branch  # or from another branch/commit
gh run watch --repo colangelo/herdr-max
```

What it does: builds the **two macOS targets only** (fast; no Linux beta), then
replaces the single rolling **`beta` GitHub prerelease** (tag `beta`, deleted +
recreated each run) and rewrites `Formula/herdr-beta.rb` in
colangelo/homebrew-tap. A `prep` job stamps **one** shared
`HERDR_BUILD_ID=<run-number>-<codename>` — the workflow run number plus the
surname of a Juventus player (2012→today), picked deterministically from the
run number by indexing the hardcoded `NAMES=(...)` array in `beta.yml`. So the
binary's `herdr --version` and the formula `version` are both exactly
`X.Y.Z-ac-beta.<run>-<codename>` (e.g. `0.7.5-ac-beta.45-zakaria`; channel
`HERDR_BUILD_CHANNEL=ac-beta`; Cargo.toml still plain `X.Y.Z`). The run number
keeps `brew upgrade` monotonic — Homebrew compares the leading number first, so
the codename is purely cosmetic. **Do not revert this to a timestamp**; the
surname scheme is intentional. Rotate the pool by editing `NAMES` in `beta.yml`.

Install / upgrade / verify:

```bash
brew install colangelo/tap/herdr-beta      # coexists with stable `herdr`
brew update && brew upgrade herdr-beta
herdr-beta --version                        # herdr X.Y.Z-ac-beta.<run>-<codename>
```

Notes:
- Needs the same `HOMEBREW_TAP_TOKEN` secret on colangelo/herdr-max as stable.
- Build ids are `<run>-<codename>` (above), monotonic via the run number.
  Changing the scheme breaks that ordering once — the 2026-07 timestamp→run-
  number switch made the first new build sort *below* the installed one, so
  `brew upgrade` skipped it. Cross a scheme change with one `brew reinstall
  herdr-beta` per machine; `brew upgrade` resumes after.
- No docs promotion, no tag, no `just check` gate — beta trusts the pushed
  `master` (run `just check` before pushing). CI on `master` should be green
  first.
- Beta is macOS-only by design; the stable `-ac` release still ships all four
  targets. Promote a soaked beta by cutting `just release-ac X.Y.Z-ac`.

## Upgrading & migrating between channels (live handoff)

herdr runs a persistent background server that owns each pane's PTY **master
fd**. A *live handoff* fork/execs a target binary as a new server and passes
those fds to it over a Unix socket (`SCM_RIGHTS`), with a two-phase commit, so
**running shells/agents survive** the swap. Direct (curl, `~/.local/bin`)
installs get this via `herdr update --handoff`; **self-update is disabled for
brew/mise/nix installs** (path-shape gate in `src/update.rs`), so those upgrade
via the package manager and then trigger the handoff themselves. `herdr` and
`herdr-beta` are separate binaries but share the **same default session/socket**
(`config_dir/herdr.sock`), so migrating one onto the other takes over the same
session in place.

### Is a handoff possible? (check before relying on it)

- **Server up + supports handoff** (Unix/macOS only; false on Windows):
  ```bash
  herdr status server --json | jq '.capabilities.live_handoff'   # want: true
  ```
- **Target binary exists + executable:** `command -v herdr-beta` (or `herdr`).
- **Same session:** both use the default session, or pass the same
  `--session <name>` to each. Different sessions = different sockets = no shared
  server to hand off.
- **`HANDOFF_VERSION` parity** between the two builds — the importer rejects a
  mismatch (`manifest.version != HANDOFF_VERSION`, `src/server/handoff.rs`).
  Maintainer check across versions:
  `git show <tag>:src/server/handoff.rs | grep HANDOFF_VERSION` (both 0.7.1-ac
  and 0.7.3 are `1`).
- **≤ 64 panes** with live PTYs (`MAX_FDS_PER_HANDOFF`); more aborts the handoff
  safely.
- **Protocol may differ** (e.g. 15→17) as long as you do **not** pass
  `--expected-protocol` (that guard, when set, requires an exact match).
- **Safety net:** the handoff is a two-phase commit — any failure *before*
  commit rolls back and the old server keeps running with panes intact. So it is
  safe to just attempt it; if it is not possible it errors and nothing is lost.

### Same-channel upgrade (pick up a newer build, keep panes)

```bash
just brew-upgrade              # brew: brew upgrade herdr + live handoff onto it
just brew-upgrade herdr-beta   # brew: same for the beta formula/binary
herdr update --handoff         # direct (curl) install only
```

### Stable -> beta

```bash
herdr server live-handoff --import-exe "$(command -v herdr-beta)"
herdr-beta                     # reattach with the beta client
```

### Beta -> stable

```bash
herdr-beta server live-handoff --import-exe "$(command -v herdr)"
herdr                          # reattach with the stable client
```

### Caveats

- **Reattach with the matching client.** After handoff the server runs the new
  binary's protocol; a client at a different protocol refuses to attach (e.g.
  after stable→beta, the old `herdr` (protocol 15) can't attach to the 0.7.3
  (17) server — use `herdr-beta`).
- **Run the handoff from a plain terminal, not inside a herdr pane.** Clients
  briefly disconnect/reconnect during the swap; there is no inside-herdr guard,
  so it works from within too, just visually confusing.
- **Downgrade (beta→stable) is the less-tested direction** (new binary's session
  state read by an older binary). The handoff itself (PTYs) is fine and the
  rollback protects you, but a later cold restart on stable reading
  beta-written `session.json` can hit format drift; prefer forward moves.

## Pre-flight

```bash
git status --short                      # must be clean (untracked .env is fine)
grep '^version' Cargo.toml | head -1    # upstream base = the X.Y.Z for the tag
just release-docs-check                 # shows which docs/next files need promoting
gh run list --repo colangelo/herdr-max --branch master --limit 3   # CI green?
```

Also confirm:
- `docs/next/CHANGELOG.md` has a non-empty `## Unreleased` section (prepare
  aborts on empty; upstream syncs it at their releases, fork content accrues
  between).
- Local `just check` needs Zig 0.16.0: `brew install zig`. `build.rs` honors
  `$ZIG`, so if `zig` on PATH is still another version, run
  `export ZIG="$(brew --prefix zig)/bin/zig"` for the session.
- Its last stage cross-lints Windows. On macOS it is skipped with a notice
  (zig cannot link its native build tools against the Windows libc there,
  AC-forks/herdr-max#83); `HERDR_WINDOWS_LINT=1` forces it. On Linux it needs
  Microsoft's SDK once per machine: `cargo install xwin --locked && just
  setup-windows-cross` (prompts to accept Microsoft's SDK license).

## 1. Promote staged docs

For every file `release-docs-check` flags (`README.md CONFIGURATION.md
INTEGRATIONS.md SOCKET_API.md CHANGELOG.md` + whatever upstream added to the
gate — read the recipe): `cp docs/next/$f $f`, then commit:

```bash
git commit -m "docs: promote docs/next to public for next release"
```

## 2. Release

```bash
export ZIG="$(brew --prefix zig)/bin/zig"   # Zig 0.16.0
just release-ac 0.7.1-ac        # full fork version, no v prefix
```

The fork recipe (end of `justfile`): validates format, requires clean tree +
new tag, runs `release-docs-check`, `changelog.py prepare` (writes the
`[X.Y.Z-ac]` section), keeps `Cargo.toml` at the BASE version, runs
`just check`, commits `release: vX.Y.Z-ac`, tags, pushes master + tag to
origin.

## 3. Watch CI

Tag push triggers `.github/workflows/release.yml`:
- **flake-check** (nix), **validate-release-inputs**, **build** ×4 targets
  (with `HERDR_BUILD_CHANNEL=ac`) gate → **release** (creates the GitHub
  Release with notes extracted from the `[X.Y.Z-ac]` changelog section)
- **update-homebrew** → writes `Formula/herdr.rb` in colangelo/homebrew-tap
  (needs repo secret `HOMEBREW_TAP_TOKEN`, a PAT with write to the tap)
- **close-released-issues** is `continue-on-error` and fails harmlessly
  (upstream bot secret); upstream's `update-latest-json` is removed in the fork.

```bash
gh run watch
```

## 4. Verify

```bash
gh api repos/colangelo/homebrew-tap/contents/Formula/herdr.rb -H "Accept: application/vnd.github.raw" | head -6
brew update && brew install colangelo/tap/herdr   # or brew upgrade herdr
herdr --version                                    # must print herdr X.Y.Z-ac
```

Also push master to the internal mirror: `git push internal master`.

## Failure recovery

| Error | Fix |
|---|---|
| `version must look like 0.7.1-ac` | Pass the FULL fork version to `release-ac`, no `v` prefix. |
| `commit your changes first` | Commit or stash; untracked-only state is fine. |
| `<file> differs from docs/next/<file>` | Step 1. |
| `Unreleased section is empty` | Nothing accrued in `docs/next/CHANGELOG.md`; add at least one entry (e.g. the upstream sync summary) before releasing. |
| `cargo update -p herdr --offline` fails | Run `cargo update -p herdr` without `--offline`, then finish the recipe steps manually (check → commit → tag → push). Do NOT rerun release-ac: CHANGELOG is already prepared. |
| `just check` fails: `failed to execute zig build ... No such file or directory` | Zig missing/not on PATH — see pre-flight. For metadata-only releases, skipping local check and trusting tag CI is acceptable with user sign-off. |
| Tag CI: `tag ... doesn't match Cargo.toml version` | Cargo.toml must equal the tag's base (strip `-ac*`). The recipe does this; manual releases must too. |
| `update-homebrew` fails on `git push` | `HOMEBREW_TAP_TOKEN` missing/expired on colangelo/herdr-max → `gh secret set HOMEBREW_TAP_TOKEN --repo colangelo/herdr-max`. |
| Build failed mid-matrix after tag push | Fix, then re-run the workflow for the same tag (`gh run rerun <id>`). If the fix needs a commit, cut `-ac.2`. |
| `brew install` gets old version | `brew update` first; formula lives in colangelo/homebrew-tap Formula/herdr.rb. |

## Versioned release docs are upstream-only (decided 2026-07-25)

Upstream v0.7.5+ ships a versioned docs system: `docs/versions/` snapshots
plus `website/scripts/docs-versions.mjs`, a `Website` workflow, and a
`update-latest-json` release job that snapshots the tagged `docs/next`,
promotes it to stable, and deploys it.

**The fork does not adopt it.** `docs-versions.mjs check` asserts
`docs/versions/manifest.json`.`current` == `website/latest.json`.`version`.
Upstream's manifest tracks their herdr.dev releases (0.7.5), while the fork's
`latest.json` is deliberately fork-scoped to its own `-ac` releases (0.7.1) —
so the two are permanently out of step and the check always fails here.

Consequences, all intentional — do NOT "fix" them by re-syncing the manifest:

- `release-docs-check` omits the `docs-versions.mjs check` line (see the
  comment in `justfile`). `just website-build`, which the same recipe runs,
  still renders and validates every version snapshot, so a genuinely broken
  docs tree is still caught.
- The `Website` workflow is **disabled** on colangelo/herdr-max (`gh workflow
  disable Website`); it runs that same check on every push touching
  `website/**` or `docs/versions/**` and would sit red forever.
- Upstream's `update-latest-json` job stays removed; the fork's own
  fork-scoped `update-latest-json` publishes `website/latest.json`.

The fork distributes through GitHub Releases + colangelo/homebrew-tap and
publishes no website, so versioned docs buy it nothing. Revisit only if the
fork starts publishing its own docs site.

## Known behavior (not bugs)

- The fork binary's self-update checker compares against upstream's herdr.dev
  manifest; `0.7.1-ac` binaries treat upstream `0.7.2` as an update and would
  self-update to an UPSTREAM binary. Brew users should update via
  `brew upgrade`; direct installs can set `[update] version_check = false`.
- Release tags point at commits that become orphaned after the next upstream
  sync (history is rebased). Tags and their releases stay valid.

Related: `.codex/skills/herdr-sync-upstream/SKILL.md` (sync before releasing),
`~/_sync/dev/CONTEXT/SKILLS/fork-maintenance/SKILL.md` (general pattern).
