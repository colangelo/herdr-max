# Each macOS pane runs as its own live launchd job

## Why

On 2026-11-16 23:40:27Z one Dock "stop" of a leftover app entry ended 12 spaces and 9 more panes (26 agent sessions)
in ac's herdr (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/288). An app launched from a pane is tied
by LaunchServices to the pane's resource coalition; once that coalition's launchd job process has exited, the app's
leftover entry stays stoppable, and stopping it ends the whole coalition. Today every pane shares one coalition with
the server and the pane anchor, and herdr lets the job exit on purpose (herdr#4920) so panes outlive logout.

Measured on m4m (#288 comments): a program that runs as a gui/<uid> launchd job which stays running leaves no
stoppable entry, and `pbcopy`, `osascript` and app launches work there. A user/<uid> job (which could outlive logout)
cannot be created without root.

**ac's decision (asks row gestore-lab.a1117-01, signed 2026-11-17 10:34:55Z): option 1.** One live launchd job per
pane; a Dock stop can end at most that pane; panes end at logout and come back by resume on the next herdr start.

## What changes

- On macOS each new pane program is started by a per-pane keeper that runs as its own gui/<uid> launchd job and
  stays that job's process for the pane's whole life. The server keeps the PTY master as today.
- A pane whose keeper ended without reporting its program's exit (logout, a stop of the pane's own coalition) is
  kept in the layout as suspended, with its resume plan, instead of being closed; the next herdr start, or the next
  client attach once a GUI session exists again, respawns it and resumes its agent.
- The live handoff replacement starts through launchd too (handoff-own-coalition, already implemented on the branch).
- Fallbacks: over SSH or with no GUI domain, panes start through the pane anchor or directly, as today.

## Not in scope

- herdr-core terminals (Herdr Core.app spawns them): same model requested from herdr-core upstream,
  https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/287.
- Linux and Windows: unchanged.
