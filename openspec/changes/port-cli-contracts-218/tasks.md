## Rules for the seat (read first)

- Work in your own worktree `~/dev/worktrees/herdr/port-cli-contracts-218` (via `wt`), branch
  `port/cli-contracts-218` from fork master after step 1 (`sync-merge/57` or later). Never in
  `~/_sync/dev/herdr`.
- **Codex sandbox:** the lead launches the seat as `codex -m gpt-6.1-sol -c
  model_reasoning_effort=high --sandbox workspace-write -C ~/dev/worktrees/herdr/port-cli-contracts-218
  --add-dir ~/_sync/dev/herdr/.git --add-dir ~/.cargo --add-dir ~/.cache --add-dir
  ~/.local/share/herdr` (without the main repo's `.git` a `wt` worktree cannot commit; the last dir
  holds the Windows cross SDK). codex itself is never wrapped in `lockf`; each heavy command is. Inside the sandbox run
  only targeted tests (`cargo nextest run -E 'test(...)'`, locked command below). The full
  `just check` fails in the sandbox (`/var/tmp`, sockets): ask for it as a line
  `NEEDS-RUN: just check` in `.local/port/PROGRESS.md` of your worktree; the lead's runner picks it
  up (the name `port-cli-contracts-218` matches its `port-*` scan) and writes
  `.local/port/runs/NNN.out`.
- **Every heavy command** (cargo build/check/clippy/test/nextest, `just check`, `just test`) runs as
  `lockf -k /tmp/m4m-heavy.lock nice -n 15 env CARGO_BUILD_JOBS=6 NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`,
  or as a `NEEDS-RUN:` line for the lead's runner. No exceptions, including a debug build.
- Never `cargo check --target *-linux-gnu` (it clobbers the vendored libghostty-vt archive).
- Commits: lowercase conventional, no co-author or session trailers, body line
  `refs https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/218`.
- Do not push to master and do not merge to master; hand the branch tip to herdr-sync (the lead),
  who runs the lead check, CI and the merge.
- Asks go to herdr-sync, never to ac.

## 1. Inventory test first (red before the merge is resolved)

- [ ] 1.1 Write the inventory as a `#[cfg(test)]` unit test under `src/cli`
      (e.g. `src/cli/fork_inventory_tests.rs`), NOT under `tests/cli`: macOS `just check` never
      compiles `tests/cli` (its cfg excludes macOS), so only a unit test gives you red/green
      locally. For each row of the design's inventory table AND each fork fix to an upstream
      command (task 4.9), a sample argv and the expected request JSON
      (`{"method": ..., "params": ...}`) or the expected custom-command parse result, taken from
      today's fork parsers at `f3fb2099` by calling them directly. Keep upstream's Linux
      `tests/cli/contracts.rs` samples as the second net (task 5.2).
- [ ] 1.2 Add a second test: the set of fork-only method names (the 27 in design.md) is a subset
      of `Method::OPERATIONS` names.
- [ ] 1.3 Commit (`test(cli): freeze the fork's cli requests before the contract port`). It passes
      on `f3fb2099`.

## 2. Merge

- [ ] 2.1 `git -c merge.conflictStyle=diff3 -c rerere.enabled=false merge 241b99cd`.
- [ ] 2.2 For every conflicted CLI/API file, take upstream's side (`git checkout --theirs`), except
      keep the fork's params structs and fields in `src/api/schema/*.rs` (`force`, todo types,
      layout preset types, sync types, pin types): resolve those by hand, keep both.
- [ ] 2.3 `src/server/client_commands.rs`: drop the hand list; the client-shell set now comes from
      `client_shell_method_names()`.
- [ ] 2.4 Keep working inside the merge (sections 3 and 4) until it compiles, then commit it once
      as `merge: upstream 241b99cd (weekly sync 2026-10-13 step 2)`; later fixes are their own
      commits.

## 3. Operations table

- [ ] 3.1 Add one `define_operations!` row per fork method (27). For each, record changes_ui and
      client_shell with the source line that justifies it (old `CLIENT_SHELL_METHODS`, old
      UI-dirty list) in `.local/port/ops-table.md` for the PR body.
- [ ] 3.2 `cargo check` (locked command above). Fix `Method` match arms the macro now requires.

## 4. CLI leaves, group by group (one commit per group)

- [ ] 4.1 todo: new `src/cli/todo_bindings.rs` with leaves for add/list/done/undone/rm/clear;
      `edit` stays custom. Wire into `src/cli.rs` and `spec.rs` like upstream's groups.
- [ ] 4.2 notification: list/clear/center into `notification_bindings.rs` (check whether `center`
      is a method call or a local action first).
- [ ] 4.3 client + popup: `client list`, `popup close [--plugin]`.
- [ ] 4.4 pane: clear, respawn, sync (on/off/toggle/link/unlink), layout presets with every alias
      (even-h, even-horizontal, even-v, even-vertical, main-v, main-vertical, tiled, balance),
      report-hint; `send-text --chunk/--chunk-delay` (options before or after the pane id, and
      the `--` cases, as the tests at f3fb2099 `src/cli/pane.rs:2185-2250` cover; port those
      tests) and `read --strip-dim` as custom wrappers.
      Add each new leaf's sample to the pane bindings test (`cases.len() == LEAVES.len()`).
- [ ] 4.5 tab: `tab sync` (--on/--off/--toggle/--tab/--current), `close --force`.
- [ ] 4.6 agent: pin/unpin leaves; `agent message` stays custom on top of the new layer.
- [ ] 4.7 workspace: pin/unpin leaves, `close --force`.
- [ ] 4.9 Fork fixes to upstream's own commands: for EVERY commit in
      `git log --no-merges $(git merge-base f3fb2099 upstream/master)..f3fb2099 -- src/cli src/cli.rs src/api`
      (66 commits), one line in `.local/port/cli-audit.md`: `<sha> <subject>: ported in <file/leaf>`,
      or `covered by <test>`, or `dropped: <reason>`. Known ones to check first: `fba7008e`,
      `c27f5838`, `10d47619`, `d091fb5f`, `d774bc63`. This list goes into the PR body and on #218.
- [ ] 4.8 Every remaining flag from `git diff 212b710b f3fb2099 -- src/cli | grep '^+' | grep -o '"--[a-z-]*"'`
      is either ported (name the leaf) or listed as intentionally dropped with a reason (none
      expected).

## 5. Verify

- [ ] 5.1 The inventory tests from 1.1 and 1.2 pass unchanged.
- [ ] 5.2 Upstream's `tests/cli/contracts.rs` passes, with fork samples added where it enumerates.
- [ ] 5.3 `HERDR_UPDATE_API_SCHEMA=1 just test-one generated_protocol_schema_artifact_is_current`
      (locked), then the frozen v1 shapes test passes with no fixture edits.
- [ ] 5.4 Full `just check` (locked) + the Linux CLI suite compile (widen `tests/cli.rs`'s cfg to
      `#![cfg(unix)]`, `cargo check --locked --all-targets`, restore; `git diff tests/cli.rs` empty).
- [ ] 5.5 CLI docs: `docs/next/website/src/content/docs/cli-reference.mdx` still lists every fork
      command; regenerate if upstream generates it now.
- [ ] 5.6 Hand the branch tip + `.local/port/ops-table.md` + `.local/port/cli-audit.md` + a one-line-per-group summary to
      herdr-sync. The lead runs the lead check, the live smoke test (each fork command once
      against a throwaway server), CI on all OS, and the merge to master.
