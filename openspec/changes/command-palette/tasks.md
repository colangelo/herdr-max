Planned 2026-10-09. Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/182. Build
after the #171 cutover, on the 0.9.3 client shell, once ac has answered the open questions in
`proposal.md`.

## 0. Planning (this change)

- [x] 0.1 read the issue, planner's send-path findings (agent-gateway § "The send path") and the
      herdr facts on 0.9.3 (help registry, `record_binding`, overlays, send-text, agent-bell, hermes)
- [x] 0.2 write the proposal, design, specs and mockups (`openspec validate command-palette --strict`)
- [ ] 0.3 ac answers the four open questions (through herdr / manager-lab); fold the answers in

## 1. Part 1: the palette (upstream candidate)

- [ ] 1.1 red tests: the command table projects to today's help groups unchanged; every table entry
      with an action is a palette row; fuzzy order for `spl`, `zoom`, `ws`
- [ ] 1.2 `CommandEntry` table in `src/input/keybind_help.rs`; `keybind_help_groups` from it
- [ ] 1.3 `KeybindAction::CommandPalette`, default `prefix+:`, help entry in `global`, `:` hint on the
      NAVIGATE bar
- [ ] 1.4 red rendered-layout tests (mockups A2, B): title + count, query row, rule, accent bar,
      dim right-aligned shortcuts, key bar; `esc` clears then closes; `enter` runs through
      `record_binding`
- [ ] 1.5 `ClientShellOverlay::Palette`, render, keys (`list_chord`, `TextEditor`), `help_lines`
- [ ] 1.6 indexed actions take a digit argument (`switch workspace 3`)
- [ ] 1.7 `just check`; dogfood with `herdr-dogfood`; keep part 1 in its own commits for an upstream PR

## 2. Part 2: send (fork only)

- [ ] 2.1 red tests for `agent.send`: piece splitting on character boundaries, Enter as its own key,
      `answer` presses the option key, `busy: interrupt` with a target that stays working sends
      nothing
- [ ] 2.2 live check in a throwaway session (herdr-throwaway-proof recipe): a typed newline inside
      the text in Claude Code (line break or submit?); decide multi-line support from it
- [ ] 2.3 `agent.send` + `AgentSent` + `AgentSendReceipt`; schema artifact and endpoint shape digest
- [ ] 2.4 `agent.directory`: herdr agents of every attached server + `directory_command`, joined on
      the Claude session id, cached 10 s
- [ ] 2.5 `prompt_options` on `agent.get` for Claude Code's question / permission / form blockers
- [ ] 2.6 the send flow in the bar (mockups C1-C3, D, E) with rendered-layout tests
- [ ] 2.7 `herdr agent send` CLI
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
