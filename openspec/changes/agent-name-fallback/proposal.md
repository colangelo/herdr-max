## Why

- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/130 (ac: "fix names",
  "put in next beta"): an agent nobody named with `agent rename` has no
  `name`, so name-based tools skip it (17 of them on mbm5; the nightly idle
  maintenance reads `name` from `agent list`) and `agent read <name>` cannot
  reach it. Claude Code already keeps its terminal title equal to its session
  name, so herdr has a good name in hand.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/131 (ac: "that's a
  good idea!"): notifications say "claude finished", which names the kind, not
  which of dozens of Claude panes finished.

## What Changes

- **Title fallback name.** An agent with no explicit name gets its stripped
  terminal title as its name when that title looks like a name: one word of
  1-64 characters from `A-Z a-z 0-9 . _ -`, starting with a letter or digit,
  that is not an agent kind or a shell name (`claude`, `codex`, `zsh`, ...).
  So "Claude Code", an empty or spinner-only title, and a task sentence
  ("fix the flaky test") get none.
- A fallback is dropped when it equals another agent's explicit name, or when
  two agents would get the same fallback. An explicit name always wins, is
  never overwritten, and is the only kind `agent rename` checks for conflicts.
- The fallback is worked out when asked, from the current title, so it follows
  `/rename`; nothing is stored or persisted.
- **API:** `name` carries the fallback, with `name_source: "title"` next to it
  (absent for an explicit name), so consumers that read `name` work unchanged.
  Agent targets (`agent read/send/keys/rename/...`, and terminal targets)
  resolve a fallback name after explicit names.
- **Notifications** are titled with the agent's name (explicit or fallback):
  "direction-builder finished"; the kind when there is none ("claude
  finished"). The same title goes to the herdr toast, the notification center,
  and terminal and system notifications. The context line is unchanged.

## Capabilities

### New Capabilities

- `agent-name-fallback`: agents without a herdr name are named from their
  terminal title, and notifications use agent names.

## Impact

- `src/terminal/title.rs`: the name rule, pure.
- `src/app/state.rs` / `src/app/agent_names.rs`: `AppState` resolves names
  (explicit first, then unique title fallbacks), O(agents) per call, only on
  API and notification paths, never per frame.
- `src/api/schema/agents.rs`: `AgentInfo.name_source`.
- `src/app/agents.rs`, `src/app/terminal_targets.rs`: list and targets.
- `src/app/actions.rs`, `src/server/notifications.rs`: notification titles.
- Docs: `agents.mdx`, `socket-api.mdx`.
