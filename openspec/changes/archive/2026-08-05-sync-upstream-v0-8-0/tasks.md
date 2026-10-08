# Tasks — sync-upstream-v0-8-0

## 1. Assessment (done during proposal)

- [x] 1.1 Fetch upstream; measure divergence (112 up, 218 fork, base `d4e0dd3d`)
- [x] 1.2 Read v0.8.0 rationale; identify the three render optimizations
- [x] 1.3 Map dual-touched files (53) and heaviest overlaps
- [x] 1.4 Confirm timer deletion upstream and `ListMotion::next_due()` fit
- [x] 1.5 Detect protocol collision (fork 19 vs upstream 19)

## 2. Rebase (scratch branch, in the sync worktree)

- [x] 2.1 Rebase `--onto upstream/master d4e0dd3d` — 219 picks replayed,
      215 landed (4 docs commits became no-ops), zero upstream file deletions
- [x] 2.2 Infra conflicts resolved per skill §2; upstream's expanded
      `update-latest-json` (RELEASE_DEPLOY_KEY) kept removed; justfile
      `release-docs-check` keeps the fork omission plus upstream's new
      `docs-preview.mjs check`
- [x] 2.3 Changelog resolved by deterministic reconstruction
      (`reconstruct_changelog.py`): upstream skeleton + fork `-ac` release
      sections byte-identical + fork-only entries merged into Unreleased
      under their fork headings; contamination the auto-merge folded into
      released sections repaired; both duplicate checks pass and the
      reconstruction is a fixpoint of the final file
- [x] 2.4 Spinner-arming fork hunks dropped during replay
      (`next_animation_tick`, `sync_animation_timer*`,
      `agent_panel_has_animation`, `has_working_pane` at both levels,
      `spinner_frame`/`SPINNERS`, `state_dot`/`agent_icon`); zero residue
      by grep
- [x] 2.5 Fork-scoped `website/latest.json` kept; `just latest-json-check`
      green against the live manifest

## 3. Motion port (design D1)

- [x] 3.1–3.3 Not needed as new work: the fork's motion was already
      deadline-driven — `sort_motion_next_due()` feeds
      `next_headless_loop_deadline` and `advance_sort_motion` runs in the
      scheduled-task path on both attached and headless servers; the hunks
      survived the rebase intact. Only the spinner used the deleted
      periodic timer.
- [x] 3.4 Covered by the fork's existing pins (`ListMotion` unit tests,
      `workspace_entries_hold_order_until_motion_ticks`,
      `agent_panel_target_keys_match_priority_entries_order`);
      `sort_motion_next_due` is `None`-gated on `sort_motion_bubble`
- [x] 3.5 Timer/spinner residue grep clean

## 4. Glyph re-layer (design D2)

- [x] 4.1 Upstream's `state_icon`/`state_icon_symbol` (static marks +
      `ui.status_indicators` styles) kept as the only glyph source; fork
      spinner-styling variants dropped at every call site
      (sidebar/mobile/navigator/status)
- [x] 4.2 `[ui.state_colors]` threaded through upstream's `state_icon` via
      `StateIconColors`; fallback slots verified identical to upstream's
      `state_label_color` palette choices (yellow/green/teal/red/overlay0)
- [x] 4.3 Layers remain separate commit series on the rebased history
      (editorial style, state colors, jump numbers, working-display-state)
- [ ] 4.4 Live render check of the four layers in a running herdr (AC
      drives; HITL per ticket #50)

## 5. Protocol (design D3)

- [x] 5.1 `PROTOCOL_VERSION` bumped to 20
- [x] 5.2 `tests/cli/sessions.rs` expectations updated; remaining literal
      19s verified to be frozen wire-layout/serde fixtures, not version pins

## 6. Verification

- [x] 6.1 Fork-surface check per skill §4 green (no deletions; release.yml
      fork hunks present, upstream deploy-key job absent; `release-ac`
      recipe; workflows parse)
- [~] 6.2 Handoff-resurface covered by the replayed fork test suite;
      live resurfaced-pane render check pending (AC, with 4.4)
- [~] 6.2b Todo-panel/notification-center hit-testing covered by unit
      tests; live hover check pending (AC, with 4.4)
- [x] 6.2c U21 landed cleanly: the fork's dual-ranking module replaced
      upstream's done-unseen-beats-working pin during replay;
      `display_state_working_beats_done_unseen` (+ tab variant) pin the
      fork behavior
- [x] 6.3 `just check` fully green: fmt, clippy `-D warnings`, 3398
      nextest tests, 101 maintenance tests. Post-rebase fixes: test-support
      `CURRENT_PROTOCOL` and the intentional ping pin to 20, schema artifact
      regenerated, sidebar tests pinned to the fork layout, and the U9
      copy-mode repeat fix re-expressed in the input-lease context
      (`TerminalInputContext::Copy`). The skill's known upstream-baseline
      flake (`live_handoff_keeps_unmanaged_agent_name...`) passed this run.
- [x] 6.4 Changelog duplicate checks green (part of reconstruction fixpoint)
- [x] 6.5 U9 re-expressed in the lease model in this change after all
      (`de4ccfbd`): `TerminalInputContext::Copy` gives copy mode a stable
      context so held keys reprocess; the context-transition guard stops
      repeats for keys that leave copy mode; headless routing gained a
      `routes_to_terminal` guard so Copy-context keys stay on the app-level
      path. refs https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/9

## 7. Adopt and post-sync (skill §5–§6)

- [ ] 7.1 Fast-forward `master` to the reconciled branch; force-push with
      lease to `origin` and `internal`
- [ ] 7.2 Disable any new upstream bot workflows on `colangelo/herdr`
      (watch for renames after the `herdrdev` org transfer)
- [ ] 7.3 CI green on pushed master; `just latest-json-check`
- [ ] 7.4 Drift check: upstream changes to `justfile` release recipes,
      `scripts/changelog.py`, `release.yml` → update herdr-release skill and
      `release-ac` if the flow moved; update the CONTEXT PROJECTS entry
- [ ] 7.5 Remove the task worktree and branch after integration

## 8. Contribution-effort handoff (ticket #50 resolved-when; do not re-run research)

- [x] 8.1 Post-rebase unit→SHA mapping generated (214 of 220 commits map
      1:1 by subject; 6 docs commits became no-ops — superseded by the
      changelog reconstruction or upstream's own translations); to be
      recorded on an `internal` research branch with the tag
      `wayfinder/pre-v0.8.0-rebase` kept
- [ ] 8.2 Close https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/50
      with the mapping; re-validation of the contribution findings resumes
      as ticket #51 (see the resume brief on
      `internal/research/upstream-proposal-pack-resume-brief`)

## 9. History blemishes — RESOLVED

Two replay ranges briefly carried a broken tree (missing brace in
`src/config/model.rs` at picks 27-41; a staged conflict-marker block in
`src/ui/sidebar.rs` tests at picks 118-124). A surgical `rebase -i` pass
repaired both source commits before adoption; the final tree is
byte-identical to the fully verified pre-pass tip and every one of the 225
replayed commits is marker-free and parses at the repaired files.
