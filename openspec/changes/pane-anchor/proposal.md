## Why

On macOS, a privacy prompt raised by a process in a pane ("herdr-beta would like to
access data from other apps", the TCC AppData service) is attributed to the pane's
*responsible process*, and the answer is kept only while that process lives. Today
the responsible process of every pane is the herdr server that spawned it. Since
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/240 a live-handoff
replacement server is its own responsible process, so every upgrade leaves all open
panes hanging off a dead server: each `op` call in a shell raises the prompt again,
and the prompt is silent, so `op` looks hung. Restarting every pane after every
upgrade is the only workaround.

ac, 2026-10-29: "again same issue, every 1p prompt triggers the herdr-beta access to
other apps... isnt there a way to avoid this issue at every update", then "the
problem with that prompt is thats silent, whereas with 1p i get a watchid buzz" and
"go real fix in herdr". Tracked as
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/246.

## What Changes

- A **pane anchor** on macOS: a small long-lived process per herdr session, started
  by the server with responsibility disclaimed (the #240 spawn), that starts pane
  processes on the server's behalf. It outlives live handoffs, so the panes it starts
  keep a live, signed responsible process across herdr upgrades, and an answer given
  once stays given until the anchor ends (logout, reboot, or the session's last pane
  closing).
- The server still opens the PTY and keeps the master side. It passes the slave side
  and the command to the anchor, which forks, sets up the session and controlling
  terminal exactly as the PTY library does, and execs. The anchor reaps its children
  and sends each exit status back, so pane exit handling is unchanged.
- A replacement server after a live handoff connects to the running anchor. Panes
  handed over keep running under it; new panes are started by it too.
- When the anchor cannot be started or answers with another protocol, the server
  spawns panes itself as today and logs why. Linux and Windows are unchanged.
- The vendored `portable-pty` gains one function that builds a slave PTY from a
  received descriptor, and its `serde_support` feature is enabled for the command.

## Capabilities

### New Capabilities

- `pane-anchor`: macOS pane processes are started by a long-lived per-session
  anchor so their privacy answers survive server upgrades.

## Impact

- New: `src/platform/macos/pane_anchor.rs` (anchor loop, protocol, client), a hidden
  `__pane-anchor` subcommand.
- Changed: `src/pty/backend/unix.rs` (spawn through the anchor on macOS),
  `src/platform/macos.rs` / `src/platform/mod.rs` (wiring), `vendor/portable-pty`
  (one function, tracked in `vendor/portable-pty.patches.md`), `Cargo.toml`
  (`serde_support`).
- No wire, API, session-file or handoff-manifest change. No new dependency.
