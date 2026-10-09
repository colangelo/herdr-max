# Herdr Max fork inventory

What this fork carries on top of upstream [herdrdev/herdr](https://github.com/herdrdev/herdr),
grouped by topic, with where each topic stands. Fork-only file; it never goes upstream.

## How the history is laid out (decided 2026-10-08, #171)

`master` is merge-based: the fork's own history, then one merge of upstream v0.9.3
(the merge commit on `sync/merge`: parent 1 = fork `26537d32`, parent 2 = upstream
`4f896118`), then fix commits, then the port branches merged in. It is not rewritten into a
topic-ordered linear series: the v0.9.3 sync had to be a whole-tree merge (a commit-by-commit
replay spliced code, and upstream moved the whole TUI into `src/client/shell/`), and re-deriving
~650 legacy commits as a clean series would take days for no reader. So topic order lives in
this file, not in the history. Upstream intake is closed to non-approved contributors, so legacy
commits are not made PR-shaped; when one topic should go upstream, cut a topic branch from
`master` for it then. Revert point before the cutover: tag `revert-point/pre-0.9.3` = `26537d32`.
Record: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/171

New authored commits carry trailers (read with `git interpret-trailers --parse`):

```text
Fork-Topic: <topic>            one of the topics below
Fork-Change: hm-<topic>-NN     stable id, kept when the change is replayed
Upstream: candidate | fork-only | <upstream PR URL>
```

## Weekly upstream sync (#173)

`git fetch upstream && git merge upstream/master` on a `sync/<date>` branch, fix conflicts
and `just check`, then update this file: move topics upstream now covers to **Superseded**
(name the upstream commit) and note new divergences. Skill:
`.claude/skills/herdr-sync-upstream/SKILL.md`.

## Topics

Status: **carried** (fork behaviour, present), **port** (re-implemented on the client shell in
the v0.9.3 sync), **superseded** (upstream does it; fork code dropped), **divergent** (a
deliberate difference from upstream's contract or names).

| Topic | Status | What | Where / commits |
|---|---|---|---|
| client-handoff | port | the client stays attached across a live handoff and re-execs onto a newer server build | `c0451a99` (fork f466236f, 91b9831a, 87690e3b, 7a3a6fba; issues 94, 165) |
| agent-hints | carried | claude mod question hint raises blocked ahead of the screen, rings and toasts | `998ca4ee` (fork a9c099df) |
| todos | carried + port | pane todos, board, editor; close/respawn refuse with open todos unless `force` | server carried; client panel, editor and board merged at `sync-merge/11` (board `896aeb60`) |
| close-force | divergent | `pane.close`, `tab.close`, `workspace.close` take an optional `force`; their v1 method shapes are frozen as fork shapes in `advertised_client_shell_method_shapes_stay_at_the_v1_contract` | `9fa78ab2` |
| clear-scrollback | divergent | the fork's scrollback purge is `pane.clear_scrollback` (CLI `herdr pane clear`); upstream `pane.clear` clears the screen | `9fa78ab2` (fork a81f9743) |
| update-manifest | divergent | updates read `distribution/latest.json` on the fork repo; the release flow also writes `website/latest.json` for 0.8.x-ac binaries | `c42f9068` |
| claude-integration | carried | fork hook events (UserPromptSubmit, Stop, SubagentStop skip) in the claude hook; integration version 11 | `9fa78ab2` |
| release-channel | carried | `-ac` releases, beta channel, own Homebrew tap, ZIG pin, macOS windows-lint skip | `c42f9068` |
| claude-background-shell | superseded | a Claude pane whose only activity is a background shell used to read working; #172 dropped that rule (back to upstream 987b070f, #3468: such a pane reads idle) and shows the background work as the braille `background_mark` instead | `db30c77a`, replaced by `feat/background-work-mark` (#172) |
| handoff-compat | divergent | handoff manifest keeps the fork layout (`agent_state` string + `hook_agent_state`) so 0.8.x-ac servers hand off into this build | `src/handoff_runtime.rs` |
| wire-protocol | divergent | the wire `PROTOCOL_VERSION` stays upstream's 22, although the fork's 0.8.2-ac also said 22 with another format: a fresh cross-version attach is refused both ways by `ENDPOINT_PROTOCOL_GENERATION` (an old client only prints "server closed connection"), and live handoff re-execs old clients (proofs 2-5 on #171). Bump only with upstream | `sync-merge/14` |
| input | port | copy/scroll mode, navigate keys, layout bindings, sync-panes input, mouse, help entries | `port/input`, merged through `bf8a056e` (`sync-merge/11`) |
| chrome | port | sidebar, tabs, pane chrome, toasts, state colours/symbols, pins, resize labels, notification/todo indicators (click opens their panels) | `port/chrome`, merged `3e4827ff`, clicks `7e363fa4`/`3ccbf1e3` (`sync-merge/11`) |
| overlays | port | todo panel/editor/board, notification center, confirm dialogs, move picker, navigator deltas | `port/overlays`, merged through `sync-merge/11` |
| sync-frame-hold | port | upstream holds mid-sync surfaces; the fork adds the 200 ms cap so a stuck app cannot freeze a pane | fork 839473d8 (issue 126) |
| sync-hold-cursor-test | divergent | upstream's Windows-only test `cursor_settle_ignores_intermediate_synchronized_frame_positions` (src/pane/terminal.rs) expects no render delay inside a `?2026h` block; the fork's sync-frame-hold schedules `SYNC_HOLD_MAX` on the frame that opens one, so the test expects that on the opening frame. Keep it when syncing; drop the edit if the 200 ms cap goes upstream or the fork drops it | `ad933b48` (issue 188) |
| prefix-list | superseded | `keys.prefix` as a list | upstream 7f89b11a (fork 73a00623) |
| detached-split-size | superseded | split sizes while detached | upstream new_pane_size (fork b943d332) |
| agent-restart-group | superseded | a restarted agent in another process group is a replacement | upstream AgentJobTracker::replaced_in_front (fork 9d6ba1f8, issue 112) |
| droid-scrollback-strip | superseded | droid's ED3 scrollback-clear filter | upstream 309749ad (#4432) |
| overlay-kit | superseded | the fork's overlay kit (AnchoredPanelSpec, ButtonRow, ListCursor, TextField) | dropped by ac's decision; overlays use the client-shell idiom |

Legacy fork commits before the v0.9.3 merge stay in history as they were; the classification of
all 673 of them is `port-notes/input/commits.json` on `research/post-v0.9.3-port-notes`.
