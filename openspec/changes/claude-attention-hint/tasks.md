Proposed 2026-09-30 (JOB 15, #137 part C). **Not approved: no code until ac
approves** (via herdr-helper / gestore-lab).
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/137

## 0. Measure first

- [ ] 0.1 throwaway session: does `PermissionRequest` fire for AskUserQuestion under `--allow-dangerously-skip-permissions`? (decides PermissionRequest vs PreToolUse matcher)

## 1. Tests first

- [ ] 1.1 hook asset: attention report for main session; none for a subagent
- [ ] 1.2 state: weak idle + hint → blocked(question); strong rule wins; clears on prompt/stop/exit/strong rule/cap; stale report dropped

## 2. Build

- [ ] 2.1 installer entry + integration version bump
- [ ] 2.2 `attention` session-report field, `TerminalState.attention_hint`
- [ ] 2.3 weak-screen override and clear rules

## 3. Verify

- [ ] 3.1 throwaway: question under skip-permissions reads blocked; Esc clears
- [ ] 3.2 docs, `just check`, CI, beta
