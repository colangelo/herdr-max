# Implementing respawn-pane

`proposal.md`, `design.md` and `specs/pane-respawn/spec.md` carry the what and
the why, and the two behavioural decisions are settled with the user: respawn
re-runs the pane's recorded launch command (shell fallback), and it confirms
only when it would kill live work. Implement it, don't redesign it. This file is
only the operational things that are not in those.

## Leads that save a research pass

**The spawn half already exists and is nearly right.**
`respawn_shell_for_launch_pane` (`src/app/api.rs:552`) already reads the
terminal's cwd, takes the runtime's current size, rebuilds the launch env via
`pane_launch_env`, spawns a fresh `TerminalRuntime`, swaps it into the same
`terminal_id`, calls `clear_agent_runtime_identity_after_respawn`, refocuses,
and schedules a session save. Task 1.1 is a rename plus a target parameter
around that body — not a new spawn path. Its only caller is the
`RuntimeExitAction::RespawnShell` arm at `src/app/api.rs:221`; the enum is at
`src/app/api.rs:26` and the decision function at `src/app/api.rs:511`. Keep that
caller on `Shell` so the agent-exit path stays byte-identical.

**The command to re-run is already stored.** `TerminalState.launch_argv`
(`src/terminal/state.rs:144`, builder at `:210`) holds the argv a pane was
launched with, and is `None` for plain shells — which is exactly the fallback
condition. Nothing new needs persisting. Note the existing spawn body builds a
`PaneShellConfig` (`src/pane.rs:1459`); running an argv instead is the one real
code change in group 1, so check how the split/popup path does it rather than
inventing one — `src/app/popup.rs:172` is the shortest worked example.

**Liveness is one cheap accessor.** `child_pid() -> Option<u32>` exists on both
the pane runtime (`src/pane.rs:3025`) and the `TerminalRuntime` wrapper
(`src/terminal/runtime.rs:534`). `Some` means a live child. Do not reach for
`process-info` or a foreground-job probe for the confirmation gate.

**The confirmation pattern is established, copy it exactly.**
`confirm_pane_close_with_todos` (`src/app/actions.rs:2185`) is the model: a
pending token for the same pane *is* the user's answer, so the gate consumes it
and returns false, and the modal's accept path simply re-issues the same runtime
mutation. That is why no `force` flag is needed on the wire. The accept path is
`confirm_close_accept_via_api` (`src/app/input/modal.rs:1533`), the key handler
is at `:1598`, the dialog renders from `src/ui/dialogs.rs:1061` and sets the
token at `:1940`, and `pane_has_outstanding_todos` is at
`src/app/actions.rs:2169`.

**Do NOT add an invariant for the new token.** `confirm_close_pane`
(`src/app/state.rs:1728`) is deliberately absent from
`assert_invariants_for_test`; it stays honest through the per-pane cleanup at
`src/app/state.rs:2081`. Add `confirm_respawn_pane` to that same cleanup block.
A lone assertion for the new token would be inconsistent and would fire on
states the close token is allowed to reach.

**The keybinding is a client of the API, not a shortcut past it.** Follow how
close does it end to end: `Method::PaneClose` (`src/api/schema.rs:234`),
classified at `src/api/mod.rs:78`, named at `src/api/server.rs:474`, handled at
`src/app/api.rs:1278`, wrapped by `runtime_pane_close`
(`src/app/runtime_mutations.rs:110`), and called from the TUI by
`close_focused_pane_via_api_requires_confirmation`
(`src/app/input/navigate.rs:673`) — which reads the resulting mode to learn
whether a confirmation appeared. Respawn mirrors that shape exactly.

**Binding plumbing has six touch points**, same as any `KeysConfig` entry:
`src/config/model.rs` (field near `close_pane: BindingConfig` at `:636`, default
near `:1434`), `src/config/keybinds.rs`, `NavigateAction` plus both dispatchers
in `src/app/input/navigate.rs`, `help_entry` in `src/ui/keybind_help.rs` (a
missing help entry is treated as incomplete work per `CLAUDE.md`), and the
commented template entry in `src/main.rs` near the other pane keys (`:210`
onward). CLI goes in `src/cli/pane.rs:38` with its declaration in
`src/cli/spec.rs`.

**Protocol does not move.** Adding a method is additive; `PROTOCOL_VERSION`
(`src/protocol/wire.rs`) only bumps for incompatible wire changes to an
already-published protocol. Say so in the commit rather than leaving a reviewer
to wonder.

## Verifying it for real

Unit tests cover the primitive and the gate, but the point of the feature is
recovery, so dogfood it on `-ac-beta` (see the `herdr-dogfood` skill; the loop
is `just check` → commit → push → `just beta` → `just brew-upgrade herdr-beta`,
which hands off the running server without killing panes).

Make a pane that is genuinely wedged rather than merely idle:

```bash
herdr pane split --direction down --focus
# in the new pane
sleep 10000
```

Then check the three cases the spec cares about: a live child prompts before
respawning; a pane whose process already exited respawns with no prompt; and a
respawned command pane comes back running its original command with the same
pane id in `herdr pane list`.

The pane id staying stable across the respawn is the observable difference from
close-and-resplit, so assert it explicitly rather than eyeballing the layout.
