Planned 2026-09-28 (JOB 10 from herdr-relay); ac approved the mockups and all
four picks the same day, and it was built the same day.
Issues: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/109,
/110, /111, /112; added while building: /101, /113, /114.

## 1. Tests first (red)

- [x] 1.1 #109: `navigator_draws_the_status_column_it_measured` (`src/ui/navigator.rs`): ac's rows in a 310x56 overlay snapshot; `keyboard-shortcuts · idle` and `macos-relay-3 · idle` are drawn whole. Today: cut at 18.
- [x] 1.2 #109: `navigator_status_width_counts_the_longest_state_word`, `navigator_status_width_is_capped_at_40`, `navigator_cuts_labels_before_statuses_in_a_small_window` (100x30 and a 60-wide screen), `navigator_tab_meta_says_1_pane`
- [x] 1.3 #110 data: `move_picker_shows_the_current_tab_as_context` (listed, greyed kind, skipped by the cursor, not in the count, Enter on it impossible), `move_picker_tab_rows_carry_pane_names_and_state`, `move_picker_headings_carry_pane_count_and_state`, `move_picker_gaps_separate_spaces` (`src/app/input/navigate.rs`)
- [x] 1.4 #110 search: `move_picker_search_matches_pane_names`, `move_picker_last_visible_child_gets_the_corner_branch`, `move_picker_here_row_stays_with_its_space` (`src/app/input/navigate.rs` / `src/ui/dialogs.rs`)
- [x] 1.5 #110 look: `snapshot_move_picker` and `snapshot_move_picker_searching` rewritten to the tree; new `snapshot_move_picker_small` (100x30, scrolling); `move_picker_width_is_measured_from_tree_rows`; `move_picker_detail_line_names_the_selected_tabs_panes` (`src/ui/dialogs.rs`)
- [x] 1.6 #110 mouse: `move_picker_click_on_here_row_does_nothing`, hover never selects a gap, heading or here row (`src/app/input/mouse.rs`)
- [x] 1.7 #111: `moved_pane_old_id_resolves_after_a_session_round_trip` (move across spaces, snapshot, restore, `parse_pane_id(old)` → the moved pane); `moved_pane_old_id_resolves_after_handoff`; `report_agent_session_under_a_moved_panes_old_id_records_it` (the relay's red test, through the API handler after a round trip); `a_live_id_wins_over_a_former_id`; `closing_a_pane_forgets_its_former_ids_across_a_round_trip`; `session_without_former_ids_reads_unchanged` (`src/persist/`, `src/app/api/panes.rs`, `src/app/ids.rs`)
- [x] 1.8 #112: reproduce first, in a throwaway session on m4m with a stand-in `claude` script that fires the real hook's SessionStart report (no paid agent). Record the finding on #112. If the race is confirmed: `session_report_survives_an_older_process_exit` and `session_is_cleared_by_its_own_process_exit` (`src/terminal/state.rs`), plus `pane_run_claude_restart_keeps_its_agent_session` driving detection polls around the report. If not confirmed: stop and report to herdr-relay with the new cause.

Decided while building (recorded so review can check them):
- #110 was built before its tests, not after them; its tests (data, render,
  mouse) each check behaviour the old picker lacked. #109, #111 and #112 were
  red first.
- The picker's detail is a rule and one line carved under the list in the
  picker's geometry, not the kit's boxed detail (which needs 3 rows and is for
  text a row hides).
- #112's cause was not the suspect. Nothing wipes the new session later:
  `conflicting_same_owner_session_ref` refuses a Claude `startup` report while
  the old session is on record (upstream 92a10fc0, against nested `claude -p`),
  and the old one is cleared only when detection sees the old process exit.
  Fix: detection marks "same agent, new process group" as a replacement; a
  refused `startup` is held for 60 s and adopted on the replacement, or the
  replacement lets exactly one conflicting `startup` through. Reported to
  herdr-relay before shipping.
- #113 (background_work never republished) joined the batch at ac's request,
  as herdr-tests' commit.
- #101 joined at ac's request: the compression test now fails when
  compression does not run and measures the process footprint. Compression
  runs on macOS; RSS hid the saving (MADV_FREE_REUSABLE).
- #114 (a plugin test reading a half-written file) was found by `just check`
  and fixed; #115 (a respawn test flake) was backlogged with context.
- The throwaway proof of beta.105 (m4m, session j10proof, 310x56 and 100x30
  in a sized tmux) found #116: both dialogs cut the status column by a label
  floor their short labels never needed (`clipper-relay-3, +2` where `, +1`
  fit). Fixed on master after the beta (77da76e2); it is not in beta.105.
- Proof results on beta.105: the picker and navigator match the mockups at
  both sizes, and ac's cut statuses (`keyboard-shortcuts · idle`,
  `macos-relay-3 · idle`) draw whole in the 73-wide navigator; a pane moved
  from w5 to w4, then a live handoff, then a stand-in Claude started in it
  (shell env `w5:p2`) recorded its session on `w4:p2`; 8/8 fast `pane run`
  restarts kept the new session.
- Handoffs from builds before beta.105 are captured by the old code, so panes
  moved before the upgrade lose their old ids once more; the fix holds for
  moves made on beta.105 and later.

## 2. Build (green)

- [x] 2.1 #109: split `navigator_content_width` into label and status widths, keep both on the navigator state, drop `metadata_width`, draw the measured column, labels give way first (floor 16); `1 pane`
- [x] 2.2 #110: `PaneMoveTargetItem::{Here, Gap}`, heading and tab state, pane names; cursor skips non-destinations; search over pane names; tree prefixes over filtered rows
- [x] 2.3 #110: renderer: title, rule, heading/tab/action styles, accent selection, scrollbar, detail line; measured width with a 36-column status cap
- [x] 2.4 #111: `PaneSnapshot::former_public_ids` (skipped when empty), written from `public_pane_id_aliases`, rebuilt on restore and handoff; `parse_pane_id` live-first; drop aliases equal to a live id on restore
- [x] 2.5 #112: the fix the reproduction supports (design.md § #112)
- [x] 2.6 `just check` green; `just bench-render-scale` if a shared render path moved (the dialogs are drawn only while open, so expected not to)

## 3. Ship and prove

- [x] 3.1 Granular commits (look, navigator, ids, session), `pull --rebase`, push `origin` and `internal`, no force-push
- [x] 3.2 One beta via `herdr-dogfood`
- [x] 3.3 Throwaway session on m4m (not ac's): the picker at 310x56 and 100x30 matches the mockups; the navigator's statuses are whole; a pane moved across spaces, then a handoff, then a report under its old id lands; a `pane run` Claude restart keeps its session
- [x] 3.4 Comment SHAs, the beta and how each was checked on #109, #110, #111, #112; leave them open for herdr-relay's live check
