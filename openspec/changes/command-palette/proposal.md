## Why

ac asked for one bar that finds and runs any herdr command without remembering its shortcut, and
that can also send a message to any session (planner, the gestores, a worker) without switching to
its pane. Braindump 10 (2026-10-05, planner `docs/inbox.md` item 2); approved 2026-10-09 in the
herdr coordinator pane ("palette yes"). Issue:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/182. Planner's send-path findings
(planner `docs/ideas/2026-10-09-agent-gateway.md` § "The send path, shared with herdr's command
palette", commit 9ca9c56) shape part 2.

Today a command is reached only through its key, the help panel (read only) or the CLI, and a
message reaches another session only by focusing its pane and typing, or by `agent-bell send` from
a shell.

## What Changes

- **Part 1, the palette (upstream candidate).** One key (`prefix+:` proposed) opens a one-line bar
  at the top of the screen. Typing filters every herdr command, fuzzy, with each command's
  shortcut on the right; `enter` runs it exactly as its key would. The command list is the help
  registry, made into one table of `(action, group, label)` so the help panel and the palette can
  never disagree. A new client-shell overlay, `Palette`, built in the 0.9.3 client shell idiom.
- **Part 2, send (fork only).** Typing `send ` (or picking "send to a session") switches the bar to
  a target list: herdr's own agents on every connected server, plus the named sessions
  `agent-bell who` reports on both Macs. After a target is picked the bar shows its state, then
  takes the text and a mode:
  - **as me** (default): herdr types the text into the target pane, so the session sees ac's own
    input and can approve or decide;
  - **as a note**: an agent-bell `--wake` peer message, which carries no approval.

  A blocked target offers its prompt's own options instead of free text; a busy target queues by
  default, with interrupt as an explicit modifier. The bar shows a receipt and says what it does
  not mean. The send core is a server API method, `agent.send`, so the phone path (part 3) and the
  CLI use the same thing; the palette is only one client of it.
- **Part 3, later.** Planner's phone/iPad path (a Collie pilot, or our own thin gateway) calls the
  same `agent.send`. Nothing is built for it here; the API is shaped so it can.
- Hermes: a hermes pane is a normal target; its blockers carry no `blocked_reason`, so a blocked
  hermes pane is offered "open pane" only, never free text (see design, decision 9).

## Capabilities

### New Capabilities

- `command-palette`: one bar that lists and runs every herdr command.
- `session-send`: send text to a session, as ac or as a note, with the target's state, a busy
  policy and a receipt; one server method for every client.

## Impact

- `src/input/keybind_help.rs`: the groups are built from one command table that also names each
  entry's `KeybindAction`; `keybind_help_groups` keeps its output.
- `src/client/shell/`: a `Palette` overlay variant (`state.rs`), its render (`overlays.rs` or a new
  `palette.rs`), its keys (`overlay_input.rs`), its `help_lines` entries, and rendered-layout tests
  under `tests/`. Running a command goes through `record_binding(KeybindMatch::Action(..))`, the
  path mouse clicks already use.
- New `KeybindAction::CommandPalette` with a default binding and a help entry.
- Part 2: `src/api/schema/agents.rs` gains `AgentSendParams`/`AgentSent`; the server gains
  `agent.send` (typed delivery with chunking and a separate Enter; a note delivery through a
  configured external command) and the `AgentSendReceipt` event; the endpoint shape digest and
  `docs/next/api/herdr-api.schema.json` move with it. CLI: `herdr agent send`.
- Config: `[palette] key`, `[palette.send] note_command` (default `agent-bell`), `directory_command`
  (default `agent-bell who`), `default_mode = "as_me"`.
- Outside herdr: agent-bell lives in `ac/infra` (`tools/agent-bell`, used through CONTEXT skills);
  design § "What agent-bell would need" lists the asks for its owner.

## Open questions for ac

Each has options and my recommendation. Plain answers ("1a, 2a, 3b, 4b") are enough.

1. **The key that opens the bar.**
   a. `prefix+:` — free today, and it is tmux's own "command prompt" key. *(recommended)*
   b. A direct key without the prefix (for example `ctrl+shift+p`, the editor habit). It needs the
      terminal to pass it through and is lost to apps that use it.
   c. Both: `prefix+:` by default, a direct key you can bind in `config.toml`.
2. **Default send mode.**
   a. **as me**: herdr types it, as if you were at that pane. The session can act on it with your
      authority. *(recommended, planner's finding 1)*
   b. **as a note**: a peer message the session may act on but that approves nothing.
3. **Sending to a busy session.**
   a. Queue by default (Claude Code takes it at its next pause); interrupt only with an explicit
      key (`alt+enter`). *(recommended)*
   b. Always ask "queue or interrupt?" before sending to a busy session.
4. **"As me" to a session on the other Mac, in the first version?**
   a. Yes, through that Mac's herdr when this client is connected to it; otherwise it falls back to
      a note, and the bar says so.
   b. Not in v1: other-Mac targets are notes only; "as me" across Macs comes later.
      *(recommended: smaller first step, and every cross-Mac typed send is a master key in the
      wrong window)*
