## Rules for the seat (read first)

- Work in your own worktree `~/dev/worktrees/herdr/port-cli-contracts-218` (via `wt`), branch
  `port/cli-contracts-218` from fork master after step 1 (`sync-merge/57` or later). Never in
  `~/_sync/dev/herdr`.
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

- [ ] 1.1 Write `tests/cli/fork_inventory.rs` (unix, compiled with the CLI suite): for each row
      of the design's inventory table, a sample argv and the expected request JSON
      (`{"method": ..., "params": ...}`), taken by running today's fork CLI parser at `f3fb2099`
      (add a `--print-request`-style test helper only if one does not exist; prefer calling the
      group's request builder directly as upstream's `tests/cli/contracts.rs` does).
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
      report-hint; `send-keys --chunk/--chunk-delay` and `read --strip-dim` as custom wrappers.
      Add each new leaf's sample to the pane bindings test (`cases.len() == LEAVES.len()`).
- [ ] 4.5 tab: `tab sync` (--on/--off/--toggle/--tab/--current), `close --force`.
- [ ] 4.6 agent: pin/unpin leaves; `agent message` stays custom on top of the new layer.
- [ ] 4.7 workspace: pin/unpin leaves, `close --force`.
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
- [ ] 5.6 Hand the branch tip + `.local/port/ops-table.md` + a one-line-per-group summary to
      herdr-sync. The lead runs the lead check, the live smoke test (each fork command once
      against a throwaway server), CI on all OS, and the merge to master.
