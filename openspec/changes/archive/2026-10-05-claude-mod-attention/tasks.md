https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/157

## 1. Design (milestone 1)

- [x] 1.1 this change, `openspec validate claude-mod-attention --strict`, reviewed by herdr-helper before any code

## 2. Hint state and API

- [x] 2.1 tests first: set, replace by `seq`, ignore lower `seq`, a reloaded source with a lower counter but a later wall clock is accepted, clear, expiry at the deadline, dropped on agent change and process exit, one source wins
- [x] 2.2 `TerminalState.agent_hint`, `pane.report_hint` (schema, server name map, `request_changes_ui`), `herdr pane report-hint`, `src/cli/spec.rs`, schema artifact
- [x] 2.3 `hint_deadline` in the wake-up list, `expire_hints` in both ticks (`App` and headless), with a headless-tick test

## 3. Detector merge

- [x] 3.1 tests first: question holds against an idle and a working screen; permission raises and clears 0.5 s after the screen stops showing it; reason precedence; no hint is today's result
- [x] 3.2 the merge where the screen result is published, `blocked_spell` on the effective state, notifications unchanged

## 4. The production mod

- [x] 4.1 milestone-2 spike (result in design.md (d): deny/Esc exact via `tool.call` abort, allow has no signal, so the screen clears a permission hint): measure with a real Claude (Haiku) which of `classic.PermissionDenied` and a `ui.render` `ToolProgress`/`ToolUse` signal fire when a permission prompt is answered allow, deny and Esc; record the table on the issue and pick the earliest signal (or fall back to raise-only)
- [x] 4.2 `integrations/claude-mod/` production module: open map, serialised queue, wall-clock `seq` with a counter, 5 s heartbeat while open, a 30 minute ceiling per open id (clear, drop, one log line; tested with a fake clock), question report (no permission hint, see design (d)), question report with the abort listener, clears on `turn.complete`/`session.end`, failures only logged (one per minute), `claude plugin test` tests
- [x] 4.3 `herdr-hint.sh` helper (socket JSON, inert without `HERDR_SOCKET_PATH`/`HERDR_PANE_ID`)

## 5. Install

- [x] 5.1 tests first: version gate (below the floor, no `claude`), idempotent install, uninstall removes, marker file left out (see design (e))
- [x] 5.2 `herdr integration install claude` writes the plugin and the local marketplace, runs `claude plugin marketplace add` and `claude plugin install`, bumps `HERDR_INTEGRATION_VERSION` once from the released value
- [x] 5.3 docs (`integrations.mdx`, `cli-reference.mdx`, `socket-api.mdx`); a macos-setup request to call `herdr integration install claude` (relayed, not edited here), which says never to add a commented `#herdr-attention@herdr-local` line to its plugin manifest (its reconcile disables commented-out lines); `socket-api.mdx` and `cli/spec.rs` cover the new method; `PROTOCOL_VERSION` is not bumped (a new method and optional fields only, checked against `wire.rs`)

## 6. Verify

- [x] 6.1 `just check`
- [x] 6.2 throwaway server, real Claude on Haiku: question and permission crossed with answer, Esc and kill, plus a pane without the mod; sampled states on the issue
- [x] 6.3 beta and seamless dogfood on m4m and mbm5, the mod installed on both, live evidence from a real pane
