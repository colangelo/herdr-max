# Design — sync-upstream-v0-8-0

## Context

Fork base moves from `d4e0dd3d` to `upstream/master` (v0.8.0+, 112 commits,
repo now `herdrdev/herdr`). 218 fork commits replay on top; 53 source files
are dual-touched. Upstream's release theme is "no frames nobody asked for":

- `b01fc37e` + `81f355fa`: sidebar spinner → static status marks; the
  periodic animation timer (`ANIMATION_INTERVAL` 16ms,
  `HEADLESS_ANIMATION_INTERVAL` 128ms, `next_animation_tick`,
  `spinner_tick`) is deleted (−441 lines, mostly `server/headless.rs` and
  `server/render_stream.rs`).
- `e27a4ea4`: render requests carry their source pane; panes hidden across
  all clients' tab states parse bytes but skip frame generation.
- `547aba8e`: passive mouse motion no longer redraws (~56–60 fps saved).
- `cc9fa475`: distinct status indicators redesign the glyphs on top of the
  static marks.

Standing objective: every decision below favors keeping fork features
separable for upstream PRs.

## Goals / Non-Goals

**Goals**

- Land the rebase with the fork surface intact and `just check` green.
- Keep bubble motion, gated by the existing `ui.sort_motion` config, at zero
  idle cost — matching upstream's CPU profile even when enabled.
- Re-layer fork glyph styling on upstream's base so each layer is a clean
  candidate PR.
- Resolve the protocol-version collision.

**Non-Goals**

- No new features, no new config knobs.
- No upstream PRs opened as part of this change (they follow CONTRIBUTING.md
  later; this change only keeps the series extractable).
- No attempt to keep the fork's old spinner or its arming logic.

## Decisions

### D1: Bubble motion moves to one-shot deadlines (drop the timer, keep the feature)

Upstream deleted the periodic timer but kept its one-shot deadline
scheduler: `toast_deadline`, `config_diagnostic_deadline`,
`copy_feedback_deadline` and agent-notification deadlines are checked in
`handle_scheduled_tasks[_headless]` and folded into
`next_headless_loop_deadline` → `sleep_until_or_pending`
(`src/server/headless.rs:748–778` upstream). The fork's
`ListMotion::next_due()` (`src/ui/list_motion.rs:196`) already returns
exactly such a deadline — `Some(next work instant)` while diverged, `None`
when settled.

Port:

- Do not replay the fork commits' `next_animation_tick` /
  `sync_animation_timer*` / `agent_panel_has_animation()` hunks; that arming
  condition (any working pane) existed for the spinner and dies with it.
- Add a `sort_motion_deadline` one-shot mirroring `toast_deadline`:
  - armed from `workspace_list_motion.next_due(timing)` min
    `agent_panel_motion.next_due(timing)`, re-synced wherever target order
    can diverge (state transitions, config reload, pane focus/view);
  - on fire: tick both motions, mark render needed, re-arm from `next_due()`;
  - disarmed (`None`) when both motions are settled or
    `ui.sort_motion = "instant"`.
- Cadence during motion is the configured settle (2000ms) / step (150ms)
  timing — ~7 renders/s for a couple of seconds per reshuffle, then silence.

Consequence: idle CPU equals upstream's even with motion enabled, so the
config gate governs the *feature* (animated vs instant reorder), not a CPU
tradeoff. Per the upstream-effort's locked decisions (resume brief §3),
bubble motion and the other aesthetic layers stay fork-only — the
upstream-aligned mechanism is for sync cost and consistency, not a PR
candidate.

Alternative rejected: replaying the timer config-gated. Re-adds ~441 deleted
lines, permanent conflict magnet in `headless.rs`/`render_stream.rs`, and
would forfeit optimization #1 whenever enabled.

### D2: Glyphs — upstream base, fork layers on top

Take `81f355fa` + `cc9fa475` (static marks, distinct indicators) as the
rendering base. Re-apply fork styling as thin, ordered layers, each a
separable commit series: editorial style (`ui.sidebar_style`), per-state
colors (`[ui.state_colors]`), jump numbers/active-row border, and
working-display-state rollup. Precedent: `ee3708c5`/`c5311243` did exactly
this on upstream's token-row rework last sync. Fork commits that style the
spinner specifically are dropped, not ported.

Known collision (resume brief §6, U21): upstream pins the *opposite*
aggregate behavior in `aggregate_state_done_unseen_beats_working`
(`src/workspace/aggregate.rs`) — done-unseen outranks working — while the
fork's `c33a4e4f` shows working agents in sidebar rows. Fork behavior wins;
adapt or replace the upstream pinned test with the fork's characterization
tests (`dabe5e3c`).

### D3: Protocol version 19 → 20

Both sides bumped 18 → 19 independently; the wire formats differ. Per the
convention (bump only if current source protocol is not already greater than
the latest *released* protocol — and upstream released 19), the rebased fork
bumps to 20. Update `tests/cli/sessions.rs` hardcoded expectation to match
`src/protocol/wire.rs::PROTOCOL_VERSION`.

### D4: Rebase mechanics follow the herdr-sync-upstream skill

Scratch branch `reconcile-test`, known conflict patterns (§2), changelog
dup-section checks, fork-surface verification (§4), force-push with lease
(§5), post-sync sweep (§6). Work happens in the `sync-upstream-v0.8.0`
worktree; `master` is untouched until adoption. The org transfer
(`3563f5c5`) rewrote `website/latest.json`/`preview.json` wholesale —
the fork keeps its fork-scoped `latest.json` (upstream's rewrite of that
file loses to the fork's, verified via `just latest-json-check`).

## Risks / Trade-offs

- **Hidden-pane skip vs handoff resurface** (`e27a4ea4` vs `2c7a1792`):
  both reason about pane visibility. Risk: a resurfaced working pane is
  classified hidden and its frames skipped. Mitigation: characterization
  check after rebase — resurface flow must produce a render; add/extend a
  test at the `handoff-agent-state-resurface` seam if none survives.
- **53 dual-touched files**, worst: `ui/sidebar.rs` (414 vs 1354 lines
  changed), `app/state.rs` (134 vs 1304), `app/input/mouse.rs` (134 vs
  1234 — upstream's mouse decoupling vs fork's sidebar mouse handling).
  Mitigation: resolve toward upstream's structure per D2's layering rule;
  upstream-baseline any test failure before blaming the rebase.
- **Motion re-arm coverage**: a missed `sort_motion_deadline` sync site
  means a reorder silently never animates (stuck display order until the
  next unrelated render). Mitigation: `ListMotion` unit tests already pin
  tick semantics; add an arming test at each state-transition site listed
  in D1.
- **Changelog merge** is a known silent-corruption hazard, and worse than
  the duplicate checks alone catch: during the first rebase attempt,
  already-replayed fork docs commits **auto-merged fork entries into
  upstream's released `[0.8.0]` section without conflicting** (e.g. the
  fork's protocol-17 `layout.balance` line — upstream's own changelog has
  no such entry). Final state must satisfy: released `##` sections
  byte-identical to upstream's; fork entries only under `## Unreleased`;
  plus both §2 duplicate checks.
- **Mouse-motion decoupling vs mouse-driven fork UI** (ticket #50): the
  pane todo panel and notification center may depend on motion repaints
  that v0.8.0 eliminated. Verify hover/hit affordances on both surfaces
  still update; if they relied on passive-motion redraws, route them
  through upstream's hover-sensitive-zone path.

## Migration Plan

1. Rebase on scratch branch; resolve per D1–D4.
2. Verify fork surface + run `just check`
   (`cargo nextest run --locked --no-fail-fast`).
3. Adopt: fast-forward `master`, force-push with lease to `origin` and
   `internal`.
4. Post-sync sweep: bot workflows, CI green, `just latest-json-check`,
   release-skill drift check.
5. Record the post-rebase unit→SHA mapping for the 30-unit contribution
   inventory (ticket #50's resolved-when;
   https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/50). Keep the
   tag `wayfinder/pre-v0.8.0-rebase` (= `6afc2a5b` on `internal`) until
   the inventory is re-mapped. Re-validation of the contribution findings
   is ticket #51, not this change.

Rollback: `master` is untouched until step 3; before the push, abandon the
scratch branch. After the push, the old SHA is recorded in the lease and
recoverable.

## Open Questions

- Does upstream's `render_stream.rs` still expose a per-frame damage path
  the motion tick can reuse, or does the deadline fire request a full
  sidebar render? Resolve during the port; either satisfies the spec.
- Whether `agent_panel_motion` and `workspace_list_motion` share one
  deadline or two: start with one (min of both) and split only if arming
  logic gets tangled.
