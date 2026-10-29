Planned 2026-10-29 (ac: "go real fix in herdr", ask gestore-lab.a1028-01;
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/246).

## 1. Library

- [ ] 1.1 `vendor/portable-pty`: `unix::slave_from_fd(OwnedFd) -> Box<dyn SlavePty>`,
  patch file and entry in `vendor/portable-pty.patches.md`
- [ ] 1.2 `Cargo.toml`: `portable-pty` with `serde_support`

## 2. Anchor (tests first)

- [ ] 2.1 Protocol types and framing; test: round trip, version mismatch refused
- [ ] 2.2 Anchor loop: accept, Hello, Spawn with a descriptor, reaper, Exited to the
  spawning connection, idle exit; test: in-process anchor spawns `/bin/echo` on a
  PTY, output on the master, exit code 0 arrives
- [ ] 2.3 Client: connect or start (disclaimed, from the linked executable D2),
  `AnchoredChild`; test: kill delivers SIGHUP, a broken connection fails waits
- [ ] 2.4 Hidden `__pane-anchor --socket <path>` subcommand

## 3. Wiring

- [ ] 3.1 `pty::backend::unix::spawn_with_portable_pty` asks the anchor first on
  macOS, falls back, logs once; `HERDR_PANE_ANCHOR=off`
- [ ] 3.2 Integration test (macOS): pane responsible = anchor; same anchor after a
  live handoff; anchor gone after the session stops
- [ ] 3.3 `just check` green; Linux CLI suite compiles

## 4. Ship

- [ ] 4.1 Beta, mbm5 first then m4m; one respawn wave
- [ ] 4.2 ac answers one prompt; the next beta install brings none; close 246
