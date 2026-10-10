Planned 2026-10-09. Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/182. Build
after the #171 cutover, on the 0.9.3 client shell, once ac has answered the open questions in
`proposal.md`.

## 0. Planning (this change)

- [x] 0.1 read the issue, planner's send-path findings (agent-gateway § "The send path") and the
      herdr facts on 0.9.3 (help registry, `record_binding`, overlays, send-text, agent-bell, hermes)
- [x] 0.2 write the proposal, design, specs and mockups (`openspec validate command-palette --strict`)
- [x] 0.3 ac's four open questions: built with the recommended answer each, as config switches (design § As built); ac can still change them

## 1. Part 1: the palette (upstream candidate)

- [x] 1.1 red tests: the command table projects to today's help groups unchanged; every table entry
      with an action is a palette row; fuzzy order for `spl`, `zoom`, `ws`
- [x] 1.2 `CommandEntry` table in `src/input/keybind_help.rs`; `keybind_help_groups` from it
- [x] 1.3 `KeybindAction::CommandPalette`, default `prefix+:`, help entry in `global` (the `:` hint on the
      NAVIGATE bar is not added: it waits for the #174 bars)
- [x] 1.4 red rendered-layout tests (mockups A2, B): title + count, query row, rule, accent bar,
      dim right-aligned shortcuts, key bar; `esc` clears then closes; `enter` runs through
      `record_binding`
- [x] 1.5 `ClientShellOverlay::Palette`, render, keys (`list_chord`, `TextEditor`), `help_lines`
- [x] 1.6 indexed actions take a digit argument (`switch workspace 3`)
- [ ] 1.7 `just check`; dogfood with `herdr-dogfood`; keep part 1 in its own commits for an upstream PR

## 2. Part 2: send (fork only)

- [x] 2.1 red tests for `agent.send`: piece splitting on character boundaries, Enter as its own key,
      `answer` presses the option key, `busy: interrupt` with a target that stays working sends
      nothing
- [x] 2.2 live check in a throwaway session (debug build of c9a9b804, real Claude Code with haiku, cwd /tmp/t182,
      2026-10-10): a raw LF inside typed text is a line break in Claude Code's box, not a submit; one Enter then
      sends both lines as one message. Also proven live: `herdr agent message` typed one line into a working
      session (it queued: "ctrl+enter to send now"); the bar's send flow (prefix+: then `send `, pick, text,
      enter) showed the receipt "typed to claude in 1 piece, enter pressed"; alt+enter interrupted first
      (Claude Code showed "Interrupted") and then typed the text. Decision: multi-line stays refused in v1
      (`multi_line_send`, no new code): only Claude Code is proven, other agents may submit on LF, and the
      refusal text is clear. Lifting it later is one change in `handle_deferred_agent_send` plus a per-agent check.
- [x] 2.3 `agent.send` + `AgentSent` (no `AgentSendReceipt` event in v1); schema artifact and endpoint shape digest
- [ ] 2.4 `agent.directory`: herdr agents of every attached server + `directory_command`, joined on
      the Claude session id, cached 10 s
- [ ] 2.5 `prompt_options` on `agent.get` for Claude Code's question / permission / form blockers
- [x] 2.6 the send flow in the bar (mockups C1-C3, D, E) with rendered-layout tests
- [x] 2.7 `herdr agent send` CLI
- [ ] 2.8 ask `ac/infra` (agent-bell's owner) for `who --json`, a stable `send --json`, and the
      no-session sender path (design § "What agent-bell would need")
- [ ] 2.9 `just check`; dogfood; close the issue only after a live send in each mode

## 3. Later

- [ ] 3.1 part 3: the phone/iPad path calls `agent.send` once a device identity exists (planner's
      gateway research; Collie pilot compatibility with the 0.9.3 protocol first if ac picks it)
- [ ] 3.2 hermes: map its blockers to reasons in `hermes.toml` (approval → `permission`,
      clarification → `question`), so the bar can offer their options
- [ ] 3.3 hermes: test the palette against a live hermes pane (idle, working, each blocker)
- [ ] 3.4 matched-letter highlighting and recent-first ranking in the bar
