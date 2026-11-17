# Handoff servers get their own coalition on macOS

## Why

On 2026-11-16 23:40:27Z a Dock "stop" of a leftover app entry ended 12 spaces and 9 more panes (26 agent sessions)
in ac's herdr on m4m (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/288). An app had been launched from
a pane shell; LaunchServices tied it to the shell's resource coalition, and stopping its leftover entry ended that
coalition. herdr's server and its PTY panes were in coalition 2049, which macOS also uses for the user's
LaunchAgents, because the live handoff spawns the replacement server as a child of the old one, so every handoff
since the first server keeps that first server's coalition. A fresh server started by the client already gets its
own coalition through a one-shot launchd job (upstream herdr#4920, `src/platform/macos/launchd_spawn.rs`); the
handoff path does not use it.

ac chose a herdr fix over a hook that blocks app launches (2026-11-17 ~00:25Z).

## What changes

- On macOS, `spawn_handoff_server` starts the replacement server through the same one-shot launchd job as a
  client-started server, so it gets a coalition of its own. Over SSH, or when launchd cannot carry the command, it
  falls back to today's disclaimed spawn.
- The handoff learns the replacement's pid from the handoff socket's peer instead of from the spawn, and rollback
  signals a replacement that is not our child without waiting on it.
- New panes then inherit the server's own coalition. Panes adopted by fd keep the coalition their processes already
  have until they are respawned.

## Not in scope

- Per-pane coalitions (an app's force quit while it runs would still end the server's coalition). Measured later.
- herdr-core terminals, which live in Herdr Core.app's coalition (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/287).
