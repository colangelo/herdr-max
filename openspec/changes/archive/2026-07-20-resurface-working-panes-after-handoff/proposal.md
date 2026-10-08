# Resurface Working Agent Panes After Live-Handoff Restart

## Why

After a herdr update or protocol restart (live handoff), agent panes that were actively working come back showing Idle in the sidebar, and stay that way until the user opens every space and clicks every pane. Restore hardcodes every pane's agent state to `Idle`, the re-adopted agent TUI never repaints into the new server's terminal grid on its own, and the idle-scan throttle then keeps the detector from re-scanning until new PTY bytes or a resize arrive — which in practice only happens when the user views each pane.

Design decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/17.

## What Changes

- The handoff manifest carries each pane's live `AgentState` (Working/Blocked/Idle) so restore seeds the pre-restart state instead of hardcoded `Idle` — spinners reappear immediately after restart.
- After handoff commit, the server runs an ordered background sweep over restored agent panes: per pane, a SIGWINCH repaint nudge plus a forced detection rescan (content-seq bump + idle-skip reset), staggered to avoid a thundering herd of repaints. No client attach required.
- Panes whose agents actually finished during the restart settle back to Idle once the sweep verifies them.
- Scope is live handoff only; cold restore with `resume_agents_on_restore` relaunches agents fresh, which repaint on their own and must not be seeded as Working.

## Capabilities

### New Capabilities

- `handoff-agent-state-resurface`: Carrying live agent detection state across a live handoff and re-verifying it with a server-side background sweep so working agents resurface in the sidebar without user interaction.

### Modified Capabilities

<!-- none — openspec/specs/ has no existing capabilities covering handoff or detection -->

## Impact

- `src/handoff_runtime.rs` / `src/persist/snapshot.rs` / `src/persist/restore.rs`: handoff/persist pane schema gains the live agent state; restore seeds it for imported (handoff) panes only.
- `src/server/headless.rs`: post-commit ordered sweep replaces/extends the current one-shot first-attach nudge (`nudge_handoff_panes_on_first_client_attach`).
- `src/pane.rs` / `src/pane/agent_detection.rs`: forced-rescan hook (content-seq bump + `detect_reset` notify) usable from the sweep.
- `src/pty/actor/unix.rs`: reuse of the shrink/restore repaint nudge per pane. Unix-only; live handoff does not exist on Windows.
- Hook-authority panes (Claude Code with lifecycle hooks): verify whether `full_lifecycle_authority_active` survives handoff and ensure the seeded state or the sweep covers them.
- Integration/persist version markers: bump once relative to the latest released tag per project convention, if the schema change requires it.
