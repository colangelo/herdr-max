## Why

JOB 14 from herdr-helper, 2026-09-29. ac: after a reboot, a server restart or
a crash, every agent pane must come back "in the exact same mode as they were
at shutdown": the same agent, the same session, the same launch flags and mode.

- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/123 (item D, gap in
  the JOB 13 fix): ac's GPT sessions are Claude Code started with
  `--settings ~/.config/claude/gpt.settings.json` (aiproxy). The saved resume
  command keeps `--model gpt-6-astra` but drops `--settings`, so a restore
  starts Opus (or fails on the GPT model name). Live on 2026-09-29: 4 panes.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/127 (item E, ac:
  "solve it thoroughly"): a hand-started Codex pane (`codex -m gpt-6-astra -s
  read-only -c …`) was restored as a **claude** agent; on .110 the same pane
  has no saved session at all and would come back as a plain shell.

## Root causes (research 2026-09-29)

1. **A saved session outlives the agent it belongs to.** A reported resume
   command is dropped once another agent is detected
   (`reconcile_reported_resume`, `src/terminal/state.rs`), but
   `persisted_agent_session` has no such rule. It is cleared only on a recorded
   exit of that same agent, a hook release, or a respawn. Detection that goes
   from claude straight to codex (no shell prompt in between, or a wrapper like
   `agent-presence wrap`) records no exit, and a hook conflict deliberately
   keeps the old session. So the claude session stays and restore runs it.
2. **Codex's own report is then refused, silently.** The SessionStart report
   is rejected while detection still says claude, and on an owner conflict
   unless detection has already confirmed codex. The hook usually beats the
   process probe. The API answers `ok` anyway, so nothing shows in the log.
3. **Nothing compares the saved agent with the running one**, at save or at
   restore.
4. **Launch flags come from a `ps` line.** The Claude hook splits
   `ps -o args` on spaces (quoting lost), and only for Claude; for Codex nothing
   records the flags at all. The server already reads each agent process's
   exact argv (`KERN_PROCARGS2`, `ForegroundProcess.argv`).

## What Changes

- **The server records each agent's launch flags.** When a pane's detected
  agent changes (or its agent process is replaced), herdr reads that process's
  exact argv once (through a recognised wrapper too), keeps only the flags on
  that agent's carry list, and stores them with the agent's name. They are
  saved in `session.json` (new optional field; older files load unchanged).
  - claude: `--settings <file>`, `--add-dir <dir…>`, `--mcp-config <cfg…>`,
    `--strict-mcp-config`, `--agent <name>`, `--plugin-dir <dir>` (repeatable),
    `--setting-sources <list>`, `--fallback-model <m>`; plus `--model`,
    `--permission-mode`, `--dangerously-skip-permissions` and
    `--allow-dangerously-skip-permissions` as fallbacks when the hook has not
    reported them.
  - codex: `-m/--model`, `-s/--sandbox`, `-a/--ask-for-approval`,
    `-c/--config k=v` (repeatable), `-p/--profile`, `--full-auto`,
    `--dangerously-bypass-approvals-and-sandbox`, `--search`, `-C/--cd`,
    `--add-dir`, `--enable`/`--disable`, `--oss`.
  - never: the session id, `resume`/`fork`/`--last`, prompts, `-p/--print`,
    `--session-id`, `--fork-session`, `--remote` (herdr adds its own).
  Relative paths are made absolute against the agent's working directory.
- **Restore composes the command.** Base: the agent's reported resume command
  (Claude hook: `claude --resume <id>` + model, effort, permission mode) or the
  built-in one (`claude --resume <id>`, `codex resume <id>`). Then every
  recorded launch flag the base does not already carry is appended. Codex panes
  on herdr's app-server daemon keep `--remote … -C <cwd>` as today.
- **A pane never restores as a different agent than the one running at
  shutdown.**
  - When detection shows a different agent than a saved session's, the saved
    session and launch flags are dropped (the reported resume already was).
    Detection names the pane's foreground agent, so a codex that claude runs
    through its own tools does not count as a change.
  - The snapshot leaves out a session whose agent is not the detected one.
  - A SessionStart report from the agent that now runs in the pane replaces a
    session saved for another agent, even before the process probe confirms it:
    a report that names an agent detection has not caught up with is held (60
    s) and taken when detection sees that agent. A report not applied is
    logged.
- **Codex without a report still restores.** A codex started as
  `codex resume <id>` gives its id from its argv (already parsed for naming).
- **Dry run.** `pane list` / `pane get` show `restore_argv` in the agent
  session info: the exact command a restore would run now (same composer). A
  script checks it before a reboot with
  `herdr-beta pane list | jq '.result.panes[] | {pane_id, restore: .agent_session.restore_argv}'`.

Not included: upstream 6045fe6a ("persist Codex resume sessions at launch").
It only covers `herdr agent start --kind codex -- resume <id>` with no flags;
the launch record above covers that case and hand-started panes.

Integration assets: unchanged (Claude stays v9, Codex v8), unless the live
probe of the Codex hook in the tasks finds a hook-side bug.

## Capabilities

### New Capabilities

- `agent-restore`: which agent, session and flags a pane restores with.

## Impact

- `src/terminal/state.rs`: launch record, reconcile on agent change, report
  takeover, refusal logging.
- `src/detect` / `src/app`: one argv read per agent change (not per frame).
- `src/agent_resume.rs`: per-agent carry tables, the composer.
- `src/persist/snapshot.rs`, `restore.rs`: new field, save-time agent check,
  composed command.
- `src/api/schema/panes.rs`: `restore_argv` (schema regenerated).
- Docs: integrations.mdx (what is restored), socket-api / cli-reference.
