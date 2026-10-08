# Design

## Context

See proposal.md for motivation. Base `sync-merge/4` (`739732f5`) builds but retains stale test references being repaired by the lead. Runtime pins/sync/layout/respawn/history-purge and fork configuration survive; client dispatch/help do not. Original behavior comes from `sync-snapshot/2026-10-08` and the study at `research-post-v0.9.3-port-notes/port-notes/input.md`.

## Goals / Non-Goals

Restore daily-use input first, matching fork chords, defaults, mouse precedence, timings and instruction bars. Preserve upstream leases, asynchronous copy requests, pane-move identity and frozen endpoint contracts. Overlay rendering/model work is a later seat dependency. No release, push, merge or live cutover occurs in this branch.

## Decisions

1. Reuse surviving configuration/algorithms. Check letter ranges/prefix lists and local/server command manifests before adding changes. Connect non-fatal fork warnings to client diagnostics. New actions enter the existing action resolver, API mapping and effective help.
2. Copy owns local absolute cursor/selection/search state and sends existing `pane.scroll` offsets. Ctrl+k/j changes offsets while preserving/clamping the absolute cursor. Entry actions share a direction/granularity helper and explicitly adopt their destination repeat context. Other transitions retain lease suppression.
3. SCROLL is client-local, pinned to endpoint/boot/pane identity. Use a newly advertised neutral runtime scroll-intent method; do not change frozen `ClientPaneInputEvent` or add a flag. Runtime validates alternate-screen state again, uses its own encoder, drops unsupported wheel ticks, and emits matched generated key press/release if kitty release reporting is requested. Existing snapshot agent authority selects Claude Ctrl+Home/End behavior.
4. Typed sync fanout runs after normal shell input ownership checks in the server, identifies the origin's actual tab and encodes independently for peers. Keep popup/mouse paths single-target. Track original recipients for held keys so membership changes do not strand releases. SCROLL's separate method cannot enter the fanout path.
5. New runtime facts are optional named JSON controls with boot/revision guards, following agent completion/view projection. Do not add fields to frozen binary surfaces. Existing pin/sync methods are advertised, with separately frozen method shapes. Pin/menu hit helpers use endpoint-qualified stable IDs; chrome supplies visuals/order. Clear scrollback is a separately named method, preserving upstream `pane.clear`.
6. Layout mutation remains server-owned; the preset-cycle cursor is client-local, preserving fork semantics. Reuse existing runtime APIs for respawn and pane.move, respecting source identity and current client geometry reconciliation. No shared mutation relies on `ClientShellAction::Keybind`'s logging fallback.
7. Common overlay text mechanics use upstream `TextEditor`; only fork deltas (undo, modified word arrows, multiline todo behavior) extend it. Focused searchable lists prioritize Ctrl+j/k movement, while ordinary fields retain kill-to-end. Actual notification/todo/move picker overlays wait for their owner.

## Risks / Trade-offs

- Protocol/identity/input projection changes are refactor-risk → name existing lease, copy, endpoint codec/method, geometry and headless input characterization tests before editing; request a read-only roundtable before new protocol/fanout work.
- Synthetic scroll race or accidental broadcast → validate alt-screen in runtime and use the new method; tests cover lost screen, unsupported wheel and kitty release encoding.
- Membership changes strand held input → test press/repeat/release recipients across group/focus/popup changes and disconnect.
- Source builds while tests are stale → lead explicitly permits binary check plus authored tests until `sync-merge/5`; rebase then run targeted tests and `just check`, never claim an unrun test passed.
- Chrome helpers shared across branches → smallest named input helpers and progress pointers; lead deduplicates at merge. Preserve hidden-pane/presentation early exits.

## Migration Plan

Commit this plan first, then one coherent feature per commit with prescribed Fork-Topic/Fork-Change/Upstream/issue trailers. Rebase the branch onto the lead's test-clean tag before final verification. Record topic SHAs/checks in `.local/port/PROGRESS.md` and unavoidable visual differences in `.local/port/DIFFERENCES.md`. Lead owns integration/cutover/rollback.
