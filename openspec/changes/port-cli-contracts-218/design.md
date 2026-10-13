## Context

Upstream 241b99cd: 45 files, +6888/-5631. New pieces:

- `src/api/schema/operations.rs`: `define_operations!` generates `Method` (serde tag `method`,
  content `params`), `Method::OPERATIONS`, `Method::operation()`, and
  `client_shell_method_names()` from one table in `src/api/schema.rs`:
  `Variant(Params) => "name", changes_ui, client_shell;`.
- `src/cli/projection.rs`: `cli_projection!` (params struct <-> clap args), `Field`, `Input`,
  `Leaf`, `Choice`, `parse_choice`, `group_request`.
- Per group: `src/cli/{pane,agent,plugin}/bindings.rs`, `src/cli/{tab,workspace,worktree,
  notification}_bindings.rs`: a `LEAVES` table, `command()`, `request()`.
- Per-param projections live next to the params in `src/api/schema/*.rs`
  (e.g. `TabCreateProjection`).
- `tests/cli/contracts.rs` (773 lines): every leaf parses to the expected method JSON; the pane
  bindings test asserts one sample per leaf (`cases.len() == LEAVES.len()`).

Trial merge of 241b99cd onto `f3fb2099` conflicts in: `src/api/mod.rs`, `src/api/schema.rs`,
`src/api/schema/{agents,panes,workspaces}.rs`, `src/api/server.rs`, `src/cli.rs`,
`src/cli/{agent,notification,pane,runtime,spec,tab,workspace}.rs`,
`src/server/client_commands.rs`, `tests/cli/mod.rs`.

## Inventory: what the fork adds (measured against upstream 212b710b)

Fork-only API methods (27): `agent.message agent.pin agent.unpin client.list layout.apply_preset
layout.balance layout.set_preset notification.clear notification.list notification.mark_seen
pane.clear_scrollback pane.report_hint pane.respawn pane.scroll_application pane.sync
pane.sync_pair plugin.popup.close tab.sync todo.add todo.clear todo.list todo.remove todo.update
workspace.pin workspace.unpin`, plus the events `notification.posted` and `todo.changed` (events,
not methods: they stay in the event enums, not in `define_operations!`).

Fork-only CLI (from `src/cli/spec.rs` and the hand parsers):

| Group | Command / flag | Method | Port as |
|---|---|---|---|
| todo | `todo add/list/done/undone/edit/rm/clear` (`--pane --text --note --priority --link --unlink --done --all --json --yaml`) | `todo.*` | new `src/cli/todo_bindings.rs`; `edit` opens a local editor: custom command next to the leaves |
| notification | `notification list/clear/center` (`--json`) | `notification.list/clear/mark_seen` | leaves in `notification_bindings.rs`; check `center` (opens the TUI panel?) before choosing leaf vs custom |
| client | `client list` | `client.list` | new `client_bindings.rs` (or a leaf in an existing group if upstream has one) |
| popup | `popup close [--plugin ID]` | `plugin.popup.close` | leaf in `src/cli/plugin/bindings.rs` |
| pane | `pane clear` | `pane.clear_scrollback` | leaf (keep the name `clear`; upstream `pane.clear` is the screen clear, fork topic `clear-scrollback`) |
| pane | `pane respawn [--force]` | `pane.respawn` | leaf |
| pane | `pane sync` (`--on --off --toggle --link --unlink`) | `pane.sync`, `pane.sync_pair` | leaf per method, or custom if one subcommand picks between the two |
| pane | `pane layout --preset even-h/even-v/main-v/tiled/main-vertical`, `balance` | `layout.apply_preset/set_preset/balance` | extend upstream's `layout` leaf or add leaves; keep every alias |
| pane | `pane report-hint --kind permission/question ...` | `pane.report_hint` | leaf |
| pane | `send-text --chunk N --chunk-delay MS` (`parse_pane_send_text_args`, f3fb2099 `src/cli/pane.rs:1255`; options accepted anywhere after the pane id AND before it, plus the `--` cases: tests `pane.rs:2185-2250`), `read --strip-dim` (`pane.rs:621`) | (client-side) | custom wrappers around upstream's `send-text` / `read` leaves: chunking and dim-stripping happen in the CLI, not on the wire; keep every argument order those tests cover |
| tab | `tab sync` (`--on --off --toggle --tab --current`) | `tab.sync` | leaf in `tab_bindings.rs` |
| agent | `agent message` (`--interrupt --input-box ...`), `agent read --strip-dim` (`agent.rs:962`) | `agent.message` (+ reads) | custom command (`src/cli/agent_message.rs` is multi-step); `--strip-dim` as on `pane read` |
| agent | `agent pin/unpin` | `agent.pin/unpin` | leaves in `src/cli/agent/bindings.rs` |
| workspace | `workspace pin/unpin` | `workspace.pin/unpin` | leaves in `workspace_bindings.rs` |
| workspace/tab/pane | `close --force` | `*.close` with `force` | add `force` to the close projections (fork topic `close-force`) |
| server | `server status` | (local) | custom |
| any | `--remote-bridge`, `--group`, `--source`, `--seq`, `--ttl-ms`, `--answer`, `--agent` | per command | find each in `git diff 212b710b f3fb2099 -- src/cli` and port with its command |

`pane.scroll_application` has no CLI (client shell only): table row only.

**Fork fixes to upstream's own commands** are not in the table above (it lists fork-only
commands) and are the easiest to lose when a conflicted file is taken from upstream. Measured
examples: `fba7008e` agent start past its deadline reports `timeout` (`src/cli/agent.rs`),
`c27f5838` `agent list --json` and the `server status` pointer, `10d47619` `pane
report-agent-session` output (`src/cli/pane.rs`), `d091fb5f` `plugin pane open --placement popup`
in help and completions (`spec.rs`), `d774bc63` the `todo add` form message. There are 66 fork
commits on `src/cli`, `src/cli.rs` and `src/api` since the merge-base with upstream; task 4.9
accounts for every one.

## Decisions

- **Take upstream's layer, then re-add fork rows.** Resolve every conflicted CLI file to
  upstream's version first, then port fork commands as new rows/leaves. Never splice old fork
  parser code into the new files. Rationale: the v0.9.3 lesson; spliced hunks compile and lose
  behaviour.
- **Flags per method, from today's behaviour.** `client_shell` = the method is in today's
  `CLIENT_SHELL_METHODS` (after #209: agent/workspace pin/unpin are in). `changes_ui` = today's
  server marks the UI dirty after it (read `src/api/server.rs` / the old list). Write a table in
  the PR body: method, changes_ui, client_shell, source line.
- **No wire change.** Params structs stay; projections are added beside them. The frozen v1
  shapes test must pass unchanged; a failure means fix the fork side, never the fixture.
- **Custom commands stay where they are** (todo edit, agent message, send-keys chunking, read
  --strip-dim, server status), wired into the group's dispatcher before the leaf lookup.

## Risks

- A fork command silently disappears: guarded by the inventory test below.
- `changes_ui` wrong: the TUI does not refresh after a CLI command. Guarded by comparing with the
  old list in the PR table and the live smoke test.
- Upstream's contract tests (`tests/cli/contracts.rs`, the pane `cases.len()` test) need fork
  samples for every new leaf.
