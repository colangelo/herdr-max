## 1. Process info (issue 90)

- [x] 1.1 Nested foreground job and `shell_at_prompt` in `pane process-info`; schema; tests

## 2. Last input time (issue 91)

- [x] 2.1 Runtime stamp on input methods; untracked variants for the harvest and resume
- [x] 2.2 `last_input_at_unix` on pane info; saved and restored in session.json; tests

## 3. Codex thread identity (issue 92)

- [x] 3.1 Naming job reports the resolved thread; app records it as the agent session; tests

## 4. Ship

- [x] 4.1 `just check` green
- [x] 4.2 Beta; live proof on m4m in a throwaway session

Verified 2026-09-28 on m4m with the `0.8.2-ac-beta.100-alexsandro` release binary in a throwaway atuin-wrapped session: process-info saw `sleep` behind atuin and `shell_at_prompt` flipped; `last_input_at_unix` moved on `pane run`, stayed null through `pane read`, and survived a restart; a daemon Codex pane carried its thread as `herdr:codex` agent session across a restart.
