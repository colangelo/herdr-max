## Why

ac, 2026-11-21: "complete step 3 and start step 4" of the herdr-core migration
(`openspec/changes/herdr-core-client/design.md` § 6.4). Phase 2 shipped the core backend and most of phase 3's
first half: core panes are re-adopted by address after a restart or a live handoff (core-backend D10), and mixed
PTY and core sessions restore (pane-launchd-jobs V7). Four things stand between today and "core is the runtime of
every pane on both Macs":

1. **One quit ends every core pane** (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/287). Every
   program in a core terminal is a child of one holder process, `Herdr Core.app/Contents/MacOS/herdr-core`.
   Quitting it (Activity Monitor, a crash) finishes every terminal with `holder_lost`, and herdr closes each pane
   as an interrupted exit. Measured for this change on hc 0.1.21 in a throwaway session: SIGTERM or SIGKILL of
   the holder finished all its terminals with `holder_lost`; a SIGKILL of the core server left the holder and its
   programs running, and the next core reconciled them.
2. **Herdr Mobile reads the default core session** (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/293,
   step 1b). herdr-max puts its panes in core session `herdr` (core-backend D2), so the phone sees them only
   through a `~/.zshenv` bridge that rewrites `herdr-core connect` to `HC_SESSION=herdr` on m4m and ac-mbm5.
3. **The CPU gate is thin.** V12 met "our server within 1.3x of the PTY" with a worst pairing of 1.34x; the
   issue 282 re-measure found 1.0x to 1.26x depending on machine load
   (`openspec/changes/core-backend/v12-results.md`). It must be measured again before core is the default.
4. **Core is not yet every pane's runtime.** Restored panes always start on the PTY (core-backend D1), and panes
   that predate `runtime = "core"` stay PTY until someone respawns them by hand.

Record: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/295. Related: #280 (phase 2, closed),
#287, #288 (pane keeper jobs and the Suspended pane state), #293.

## What Changes

- **A core loss suspends panes instead of closing them.** A core pane whose terminal finished because the runtime
  ended it (`holder_lost`, `host_shutdown`, `server_stop`, `stopped_by_restart`, or gone after a reconnect) keeps
  its place as Suspended and starts again by its saved plan, the way a lost pane keeper does (#288 D3). Agents
  resume with `--resume`.
- **herdr-max starts its core again after a loss.** When the core's socket stays missing past a restart's
  window, the server starts the core again with the clean environment of core-backend D2. A crash-loop guard
  stops after three losses in ten minutes and resumes panes on the PTY with one notice.
- **What only upstream can do is separated.** Per-terminal responsibility or a holder per terminal is herdr-core's
  code. It is pursued only as a verified, reproducible bug report that ac submits, per AGENTS.md § External
  contributor guardrail.
- **The default herdr session runs in the default core session.** Release builds put the default herdr session's
  new panes in core session `default`. Named sessions keep `herdr-<name>`; debug builds keep `herdrdev`.
  The `herdr.session` label keeps its current value, so ownership no longer depends on which core a terminal is
  in. Panes in the old `herdr` core are re-adopted from there until they end or move.
- **Core is the runtime of restored and respawned panes**, and a new server verb moves an idle pane onto the
  primary core (from the PTY or from the old `herdr` core). A driver script moves one canary first, verifies it,
  then the rest, and never the computer-use host panes.
- **The CPU gate is a verifier row** with a conditional PR (a reader that hands whole batches to the parsers) if
  the re-measure is over 1.3x.
- **The default stays per machine.** `[experimental] runtime` keeps its code default `"pty"`. Both Macs set
  `"core"`. The code default flips in phase 6, when the PTY path is deleted.

## Capabilities

### New Capabilities

- `core-loss-recovery`: what happens to core panes when the core, its holder or the login session ends them.
- `core-session-placement`: which core session a herdr session's panes run in, and how panes are re-adopted
  across sessions.
- `core-runtime-default`: restored, respawned and moved panes run on core; the idle-move rule; the CPU gate.

### Modified Capabilities

None in `openspec/specs/` yet. `core-backend` is not archived, so its `core-pane-backend` spec is not a base to
modify. These requirements supersede two of its statements: restored panes always use the PTY ("Core backend is
opt-in per new pane"), and the core session named `herdr` for the default herdr session (core-backend D2). If
`core-backend` is archived first, those two become MODIFIED deltas here.

## Impact

- Code: `src/pane/runtime_io/core_backend/` (exit mapping, session registry, connector start rule),
  `src/pane/runtime_io/mod.rs` (`CoreTerminalLink` from the pane), `src/app/pane_suspension.rs` (backend-aware
  resume), `src/app/core_terminals.rs` (orphans per session), `src/pane.rs` (restored panes), `src/app/api.rs`
  and the API schema (a new `pane.respawn_if_idle` method), `scripts/` (the move driver).
- Persisted state: none new. `core_terminal {session, address}` already carries the session; its value for new
  panes changes from `herdr` to `default`.
- Client endpoint: no new wire field. Suspended core panes reuse the optional `suspended` fact.
- Docs: `docs/next` config reference (core loss, the default session, the rollback rule), the herdr skill line
  about quitting Herdr Core.
- Outside this repo: the chezmoi `~/.zshenv` bridge on both Macs is removed at the end (relayed to the dotfiles
  owner); the old `herdr` core is stopped by ac once it is empty.
- Rollback: an older build ignores panes in core session `default` and restores them fresh, leaving their
  programs running unseen. The rollback rule from core-backend D10 extends to both sessions.
