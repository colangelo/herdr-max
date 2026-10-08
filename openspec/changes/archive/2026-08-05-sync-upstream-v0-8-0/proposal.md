## Why

Upstream herdr released v0.8.0 (now at `github.com/herdrdev/herdr`), whose
headline change is 89–95% less idle CPU from three rendering optimizations:
static status marks replacing the animated sidebar spinner (the periodic
animation timer is deleted outright), visibility-aware rendering that skips
frame generation for hidden panes, and mouse-motion decoupling. The fork is
218 commits ahead of the old base (`d4e0dd3d`) and 112 commits behind; 53
source files are touched by both sides, and the fork's bubble-motion feature
is built on the exact timer infrastructure upstream deleted to get its CPU
win. Beyond the sync itself, the fork's standing objective is to make its
features easy to contribute back upstream as PRs, so the integration must
leave each fork feature separable, not smeared across conflict resolutions.

## What Changes

- Rebase the 218-commit fork patch set onto `upstream/master` (v0.8.0+,
  including the org transfer to `herdrdev`), per the herdr-sync-upstream
  scratch-branch procedure. History SHAs change; that is expected.
- **BREAKING (internal):** drop the fork's periodic animation timer
  (`ANIMATION_INTERVAL`, `HEADLESS_ANIMATION_INTERVAL`, `next_animation_tick`,
  `spinner_tick` arming via `agent_panel_has_animation()`) instead of
  replaying it over upstream's deletion. Bubble motion is rescheduled onto
  upstream's surviving one-shot deadline system (the `toast_deadline` /
  `config_diagnostic_deadline` pattern feeding `next_headless_loop_deadline`),
  driven by the existing `ListMotion::next_due()`. Zero wakes while lists are
  settled; the existing `ui.sort_motion = "instant"` gate is retained.
- Adopt upstream's static status marks and distinct status indicators
  (`cc9fa475`) as the glyph base; re-layer the fork's editorial style,
  per-state colors, jump numbers, and working-display-state on top as
  separable commits suitable for upstream PRs.
- Bump `PROTOCOL_VERSION` to 20: fork and upstream both bumped 18 → 19
  independently, so the fork's 19 and upstream's 19 are different protocols.
- Preserve the fork's release/CI surface through the rebase (release.yml fork
  hunks, `release-ac` recipe, fork-scoped `latest.json`) and re-verify it
  against the new org naming.
- Verify the fork's handoff resurface behavior still renders resurfaced panes
  under upstream's hidden-pane render skip (`e27a4ea4`).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `sidebar-sort-motion`: motion ticks are no longer driven by a periodic
  animation timer; scheduling becomes deadline-driven with no timer wakes
  while every motion list is settled, and cadence during motion follows the
  configured settle/step timing rather than a fixed frame interval.
- `sidebar-editorial-style`: the working-state glyph is upstream's static
  mark, not an animated spinner; `[ui.state_colors]` recoloring applies to
  upstream's static marks and distinct indicators as the base glyph set.

## Impact

- Rebase surface: 53 dual-touched files under `src/`, heaviest in
  `src/ui/sidebar.rs`, `src/app/state.rs`, `src/app/input/mouse.rs`,
  `src/app/runtime.rs`, `src/ui/status.rs`, `src/server/headless.rs`.
- Deleted-infrastructure port: `src/app/mod.rs`, `src/app/runtime.rs`,
  `src/server/headless.rs`, `src/ui/list_motion.rs` call sites.
- Protocol: `src/protocol/wire.rs`, hardcoded expectation in
  `tests/cli/sessions.rs`.
- Infra files with known conflict patterns: `.github/workflows/release.yml`,
  `.github/workflows/ci.yml`, `justfile`, `.gitignore`,
  `docs/next/CHANGELOG.md` (dup-section hazard), `website/latest.json`
  (fork-scoped; upstream rewrote it wholesale in the org transfer).
- Post-sync operational checks: disable any new upstream bot workflows on
  `colangelo/herdr`, green CI on pushed master, `just latest-json-check`.
- Upstreamability: after the sync, each fork feature series should be
  extractable as `upstream/master` + cherry-picks with no fork-only infra
  entangled. The paused upstream-contribution effort resumes after this
  rebase (tickets #50/#51; resume brief on
  `internal/research/upstream-proposal-pack-resume-brief`): this change
  must record the post-rebase unit→SHA mapping for the 30-unit inventory
  and preserve the `wayfinder/pre-v0.8.0-rebase` tag. Per its locked
  decisions, the aesthetic layers (bubble motion, editorial style, motion
  easing) stay fork-only.
- Tracker: this rebase is
  https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/50.
