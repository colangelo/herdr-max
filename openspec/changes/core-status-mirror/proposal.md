# Mirror herdr's agent status into herdr-core

Tracking issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/293

## Why

ac uses the Herdr Mobile iPhone app with herdr-max on m4m. The phone connects over SSH
with `herdr-core connect` and lists herdr-core terminals; herdr-max's core panes already
appear there. The phone's Inbox shows only terminals that carry an OSC 7501 status in
core, and core learns status only from OSC 7501 bytes the program prints (no API sets
it). herdr-max's agents report to herdr's own API through hooks and detection, and print
no 7501. So the Inbox is empty.

## What changes

- For each pane on the core backend, the herdr server writes one OSC 7501 root record
  to the program's terminal device (the pty slave, as `write(1)` does) when herdr's
  agent status for the pane changes. Core reads it as program output and stores it.
- Mapping: working → `working`, blocked → `blocked` with `kind=permission`, done →
  `done`, idle → `idle`, unknown, released or no agent → `clear`. The agent label goes in
  `app` when it is a safe word.
- A record is written only when the pane's output has been quiet for 100 ms and a byte
  scanner over the pane's core output stream says the stream is outside any escape
  sequence. Otherwise the latest record waits and is retried; older ones are dropped.
- Behind `[experimental] core_status_mirror` (default false), applied at startup and on
  config reload. Turning it on mirrors every core pane's current status; turning it off
  clears what was mirrored.

## What does not change

- herdr's own status authority: herdr still ignores core status (security review M3,
  live test V8). The echoed record never becomes a hook report or a hint.
- PTY and `pane_jobs` panes. Windows (the core backend is Unix only).

## Out of scope

- Publishing workspaces and tabs as the documents the app shows as spaces (a later
  change on #293).
- Child records (`id=`), `title`, `msg` and `progress` fields.
