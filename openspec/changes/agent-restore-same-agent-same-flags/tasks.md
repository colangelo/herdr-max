Planned 2026-09-29 (JOB 14 from herdr-helper; ac: "solve it thoroughly and add
to current build"). Issues:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/123 (D),
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/127 (E).

## 0. Probe

- [x] 0.1 throwaway session: hand-started codex with the Codex hook; log whether the SessionStart report arrives and why it is refused. Found: a real `codex` started without a prompt sends no report (no thread yet, nothing to restore); with a report, it arrived 0.4 s before detection and was refused for the claude session still on record (fixed by holding it, 2.3)

## 1. Tests first (red)

- [x] 1.1 carry tables (claude, codex): kept / dropped flags, `=` forms, relative paths
- [x] 1.2 composer: claude hook argv + launch flags, codex built-in + flags, daemon codex keeps `--remote`
- [x] 1.3 guards: agent change drops the old session; nested codex keeps claude's; snapshot skips a mismatch; codex report replaces a claude session
- [x] 1.4 persistence: old session.json loads, new field round-trips; restore uses the composed command per kind
- [x] 1.5 `restore_argv` in pane list/get equals the restore plan

## 2. Build (green)

- [x] 2.1 launch record: argv read on agent change, carry filter, state + snapshot field
- [x] 2.2 composer in `src/agent_resume.rs`, used by restore and by `restore_argv`
- [x] 2.3 guards in `src/terminal/state.rs` + snapshot; refusal logging
- [x] 2.4 codex id from `resume <id>` argv when no report
- [x] 2.5 schema field, regenerate `docs/next/api/herdr-api.schema.json`

## 3. Docs

- [x] 3.1 integrations.mdx: what a restore keeps, per agent
- [x] 3.2 socket-api.mdx / cli-reference.mdx: `restore_argv`

## 4. Verify and ship

- [x] 4.1 proof: one pane per kind, server stop + restart, restored argv read back with `ps`
- [x] 4.2 `just check`, push origin and internal, notes on #123 and #127
