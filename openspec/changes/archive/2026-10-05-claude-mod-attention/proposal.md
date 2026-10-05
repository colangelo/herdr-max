## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/157 (the build of
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/137 part C), go from
ac 2026-10-03: "137 yes go".

herdr finds a Claude Code question or permission prompt by reading the screen.
An Esc'd, late or half-drawn dialog can leave the state wrong, and upstream
dropped hook-driven state for Claude in `295b09ca` for that reason: no hook
fires after Esc, and late events re-pin the state. The spike
(`aebde74f`, `integrations/claude-mod/`, review comment 18966) proved that a
Claude Code mod, which runs inside Claude Code as middleware around `next(e)`,
sees a question open and close exactly, through answer, "Chat about this", Esc
(only through the `next.signal` abort) and SIGTERM, each clearing in about 1 s.

## What Changes

- **A small neutral hint API.** `pane.report_hint` (CLI `herdr pane report-hint`)
  lets a source say "this agent is waiting on a `question` or a `permission`"
  and, later, "it is not". It is evidence for the detector, not a state: the
  agent's working and idle still come from the screen and the existing hooks.
  Hints are runtime-only and short-lived (a heartbeat keeps one alive).
- **The detector merges the hint with the screen.** While a fresh hint is open
  the pane reads `blocked` with that reason. A `question` hint is held by the
  mod's exact close, so the screen cannot un-block it early or late. A
  `permission` hint is raised by the mod but may be cleared by the screen,
  because the end of a permission prompt is not an event a mod can see.
  Without a hint, or after one closes or ages out, detection is exactly what it
  is today.
- **The production mod.** Plugin `herdr-attention` (from the spike), with an
  `isOpen`-gated reporter, a heartbeat while a dialog is open, a `tool.check`
  based permission report, and logging only on failure. It reports through a
  small helper that writes to herdr's socket, so it does not depend on which
  herdr binary name is on `PATH`.
- **One install path.** `herdr integration install claude` installs the mod
  next to the existing Claude hooks, gated on Claude Code 2.1.287 or later, and
  `uninstall` removes it. The macos-setup fleet manifest only calls that
  command; it does not carry a second copy of the mod.

## Capabilities

### New Capabilities

- `agent-hints`: a source tells herdr that an agent waits on a question or a
  permission, and herdr shows it as blocked with that reason.

## Impact

`src/api/schema/*` and `src/api/server.rs` (`pane.report_hint`, optional
fields), `src/cli/*`, `src/terminal/state.rs` and `src/pane/agent_detection.rs`
(hint state and the merge), `src/integration/*` (install of the mod),
`integrations/claude-mod/` (production mod), docs. `PROTOCOL_VERSION` is not
bumped: one new method and no changed field. A herdr without the mod, or a
Claude without it, behaves as today.

## Open points for review

Each is decided in `design.md` with a reason and is a small constant or a
one-place choice if the review disagrees: the hint TTL and heartbeat (15 s and
5 s), the screen-clear grace for permission hints (0.5 s), the choice of the
tool-running signal for a permission prompt (verified in milestone 2), and the
install mechanism (`claude plugin install` from a herdr-owned local marketplace).
