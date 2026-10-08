## Context

See proposal.md. Work starts at sync-merge/4; shared fork facts survived, server-only input/render glue was stripped. Original fork tree sync-snapshot/2026-10-08 and the chrome study are the behavioral oracle. Lead decisions in .local/port/BRIEF.md resolve all prior questions.

## Goals / Non-Goals

Goals: equal single-server appearance/behavior, pure client presentation, shared runtime fact authority, generation-one interoperability. All tier-1 features precede optional tier-2 work. Overlays and input broadcasting remain owned by their seats; this seat supplies reusable facts/hits and drawn chrome.

## Decisions

- Preserve attention sorting and add separate displayed rollups: Working beats unseen Done only when drawing aggregates. Preserve upstream per-client completion evidence/acknowledgement.
- Outer sidebar/tab/toast/label state lives in ClientShellState/Config. Live server pane rendering retains endpoint-configured borders/tint/dim exactly as instructed; no new per-client policy.
- Shared pin/todo/head/activity/sync/log facts are optional fields in endpoint snapshot JSON. Core snapshot structs are also reachable from a frozen binary ServerMessage, so direct serializable additions are prohibited. Use a serde-skipped in-memory optional facts container and an explicit JSON envelope that flattens unchanged core snapshot plus optional facts. Decode core first and facts independently; malformed/absent facts fall back. Binary fixture/digests remain unchanged. Typed maps use stable IDs; tolerant enum parsing is confined to JSON facts.
- Pin ordering is within endpoint groups; no cross-server counter authority. Rendering, indexed navigation and hits consume the same order. Active view filtering remains authoritative over eligible rows.
- Recover pure fork helper/tests when still applicable; adapt their old AppState-driven harness to client snapshots/surfaces. Keep newer conditional token styles/hiding and machine identity handling.
- Restore fork animation defaults and exact glyph/color/timing in isolated fork-only commits. Cosmetic ticks repaint client chrome only, with no server snapshot/PTY/resize requests.
- Persistent toast replaces prior visible toast as the original fork does; inspect original before wiring queue. Keep server-owned notification history/read state; overlay seat consumes optional summary/methods.
- Record any forced visible difference in .local/port/DIFFERENCES.md. Existing wire enum additions in merge base are outside this feature's authority; no further core codec mutation.

## Risks / Trade-offs

Frozen codec reachability -> explicit JSON-only envelope and byte-equivalence tests, unchanged frozen fixtures.
Multiple render/order paths -> local/aggregate and expanded/collapsed tests, stable IDs and shared helpers.
Per-cell/animation scaling -> narrow cached access, no I/O in rendering, 1/15-pane benchmark evidence.
Parallel lead repairs -> own tree only; feature tests authored now, rebase onto test-clean tag before final just check.

## Migration Plan

Commit this plan before code. Commit each topic with lead trailers and issue URL. No push/merge. Rebase onto test-clean sync-merge tag then run just check, parity smoke/performance evidence. Lead integrates and handles beta cutover; feature commits can be reverted independently.

## Roundtable

Two read-only reviewers confirmed binary snapshot reachability, three attention-ranked display rollups, conditional-token preservation, client help in src/input/keybind_help.rs, and target-specific pane rendering. Do not transplant server app.mode faint or global active-tab lookups into target-specific client presentation.
