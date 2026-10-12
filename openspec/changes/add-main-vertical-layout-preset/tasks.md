## Implementation

- [x] 1. Core red/green: focused main geometry, order, rects, single-pane no-op and focus history.
- [x] 2. API red/green: separate enum/params, handler and dispatch, advertisement, immutable old shape and new digest, generated schema.
- [x] 3. Client red/green: advertised four-step cycle, unadvertised three-step cycle and successful transient feedback.
- [x] 4. CLI red/green: main-vertical/main-v parsing and additive dispatch, --preset and retained --set.
- [x] 5. Update config/help comments and staged keybindings, CLI and socket API docs.
- [ ] 6. Validate OpenSpec, run locked offline just check and pinned clippy, review diff, commit granularly and report to coordinator without pushing.

## Validation and handoff notes

- OpenSpec strict validation passed.
- Final focused tests: 22 passed; broader affected-module tests: 67 passed.
- Schema regeneration/current-artifact check passed. Pinned Rust 1.96.1 Clippy passed.
- `just check` cannot complete in this sandbox: unchanged Ghostty graphics tests hardcode `/var/tmp`, which is not writable. Initial full run: 2 passed, 6 failed, 5000 not run, 19 skipped.
- Maintenance tests: 180 run, 13 permission errors from hardcoded `/var/tmp` writes and socket listeners. Additional architecture tests: 6 passed; integration assets: 4 passed, 14 socket-permission failures; docs contracts: 7 passed. Windows lint reported its expected macOS skip.
- Git commits are blocked because linked worktree metadata lives under `/Users/ac/_sync/dev/herdr/.git/worktrees/feat-main-stack-layout-215`, outside writable roots. Planning was validated before code edits, but could not be committed first. Coordinator must commit and rerun unrestricted checks; nothing was pushed.
- Routing admission `a-1012-03` could not be recorded because model-routing's state directory is also outside writable roots.
