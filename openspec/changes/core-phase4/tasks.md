Each numbered group from 2 to 5 is one pull request on `issue/299-core-phase4` (or a branch stacked on
it), in this order. Each ends in `just check` green, the live rows it names under `just core-live-test`
against a throwaway hc 0.1.21 session, and CI green on every OS before the next starts. Verifier rows
are in design.md § Verifier table.

## 1. Plan

- [x] 1.1 This change, validated with `openspec validate core-phase4 --strict`, committed alone
- [ ] 1.2 herdr reviews the spec before any code (ac sends it)

## 2. PR A: core's turn counts herdr's turns

- [ ] 2.1 `owed_turn` in `StatusMirror`: owed on a coalesced `working`, redundant records kept while owed, `working` plus the record in one write (verifier 1, 2)
- [ ] 2.2 `view_dirty` on `AppState`, set by session-saving changes and client focus or shown-tab changes; the server loop re-offers every pane's record once per pass (verifier 4, unit)
- [ ] 2.3 Live test: one turn per coalesced herdr turn; a read `done` pane turns `idle` in core (verifier 3, 4)
- [ ] 2.4 Check: `just check`, `just core-live-test`, CI green

## 3. PR B: herdr writes `herdr.seen`

- [ ] 3.1 `src/app/core_labels.rs`: `CoreLabels`, `CoreFact`, `LabelWrite`, `look`, `herdr_marked_seen`, the write rules of D8 (verifier 5, 6, 7)
- [ ] 3.2 `core_backend/labels.rs`: subscription on the server's core connection, framed channel routing in the client reader, `turn`/`labels`/`removed`/`finished`/`missed`/`heartbeat` parsed, `status` dropped, reconnect with a fresh snapshot, `terminal.label` writes (verifier 13)
- [ ] 3.3 `panes_on_screen` in `client_views.rs`, recomputed on `view_dirty` (verifier 8)
- [ ] 3.4 `pane.mark_seen` writes; `herdr.seen=0` in `pane_labels` (verifier 9, 10)
- [ ] 3.5 `[experimental] core_label_sync` config, reload, `config-reference.json` entry
- [ ] 3.6 Server-path live step: `herdr pane mark-seen` sets the label to the pane's turn (verifier 15, first clause)
- [ ] 3.7 Check: `just check`, `just core-live-test`, CI green

## 4. PR C: herdr reads `herdr.seen` and syncs `herdr.pinned`

- [ ] 4.1 Read rules of D9 in `CoreLabels::apply`: raise marks seen, lowering marks an idle agent unseen, snapshot raises only (verifier 11, 12)
- [ ] 4.2 Pin rules of D10: `herdr_pinned` writes from `pins.rs`, label changes pin or unpin, connect union (verifier 14)
- [ ] 4.3 Server-path live steps: a raised label makes a done pane idle; `herdr.pinned=1` pins; `herdr agent unpin` removes the label (verifier 15)
- [ ] 4.4 Docs: `docs/next/website/src/content/docs/session-state.mdx`, what is shared with the Herdr apps and the two flags
- [ ] 4.5 Security: ask the security desk for a review of the label reader (D13) before merge
- [ ] 4.6 Check: `just check`, `just core-live-test`, CI green, review answered

## 5. PR D: the `herdr-max` document

- [ ] 5.1 `DocSpec` in `spaces_doc.rs`: name and merge as parameters, one thread per document, the 1 MiB refusal (verifier 18)
- [ ] 5.2 `src/app/core_extras_doc.rs`: format 1 types and the pure build from `AppState`, wall-clock token expiry (verifier 16, 17)
- [ ] 5.3 `[experimental] core_extras_doc` config, shared schedule, `report_metadata` sets the dirty flag, off writes `null`, `config-reference.json` entry (verifier 19)
- [ ] 5.4 Live: `doc.get herdr-max` matches `doc.get herdr-app` by id (verifier 20)
- [ ] 5.5 Check: `just check`, `just core-live-test`, CI green

## 6. Roll-out

- [ ] 6.1 `just beta-local` on m4m after each merged PR; install on both Macs in the same step
- [ ] 6.2 `core_label_sync = true` and `core_extras_doc = true` in ac's config on both Macs (relayed to mac-m4 and mac-m5)
- [ ] 6.3 ac's checks on Herdr Mobile (verifier 21, 22, 23); record the results on #299
- [ ] 6.4 Close #294's "stays done on the phone" limit and #299 after verifier 21 to 23 pass in a running beta
