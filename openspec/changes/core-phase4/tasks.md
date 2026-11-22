Each numbered group from 2 to 5 is one pull request on `issue/299-core-phase4` (or a branch stacked on
it), in this order. Each ends in `just check` green, the live rows it names under `just core-live-test`
against a throwaway hc 0.1.21 session, and CI green on every OS before the next starts. Verifier rows
are in design.md § Verifier table.

## 1. Plan

- [x] 1.1 This change, validated with `openspec validate core-phase4 --strict`, committed alone
- [ ] 1.2 herdr reviews the spec before any code (ac sends it)

## 2. PR A: core's turn counts herdr's turns

- [x] 2.1 `owed_turn` in `StatusMirror`: owed on a coalesced `working`, redundant records kept while owed, `working` plus the record in one write (verifier 1, 2)
- [x] 2.2 `view_dirty` on `AppState`, set by session-saving changes and client focus or shown-tab changes; the server loop re-offers every pane's record once per pass (verifier 4, unit)
- [x] 2.3 Live test: one turn per coalesced herdr turn; a read `done` pane turns `idle` in core (verifier 3, 4)
- [x] 2.4 Check: `just check`, `just core-live-test`, CI green

As built (PR A). `StatusMirror` holds `owed_working: Option<CoreStatusRecord>`, the `working` itself, so
the owed record carries the agent's `app`. `take_due` returns a `MirrorWrite` (`owed_working`, `record`)
whose `encode` puts both records in the one `write`; `written` pays the debt, `busy` keeps it. The
longest pair (two records with a 64-byte app) is under 256 bytes (unit test). `AppState::core_view_dirty`
is set by `mark_session_dirty`, `mark_session_dirty_urgent`, `schedule_session_save` and
`schedule_session_save_now`, and by `mark_active_tab_seen` when it changed a pane (the focus-in path).
`App::sync_core_view` takes it once per server loop pass, next to `sync_core_spaces_schedule`, and with
the mirror on hands every pane its record again. Live test
`core_turn_counts_a_coalesced_herdr_turn_once`: a pane printing every 20 ms gets `working` then `done`
50 ms apart; core's turn rises by exactly 1, and a following `idle` leaves it there.

Limits: a seen change made by a path that neither saves the session nor goes through
`mark_active_tab_seen` re-offers at the next pass that does. Native 7501 reporters can step core's turn
twice for one turn (D6). CI is the coordinator's to watch.

## 3. PR B: herdr writes `herdr.seen`

- [x] 3.1 `src/app/core_labels.rs`: `CoreLabels`, `CoreFact`, `LabelWrite`, `look`, `herdr_marked_seen`, the write rules of D8 (verifier 5, 6, 7)
- [x] 3.2 `core_backend/labels.rs`: subscription on the server's core connection, framed channel routing in the client reader, `turn`/`labels`/`removed`/`finished`/`missed`/`heartbeat` parsed, `status` dropped, reconnect with a fresh snapshot, `terminal.label` writes (verifier 13)
- [x] 3.3 `panes_on_screen` in `client_views.rs`, recomputed on `view_dirty` (verifier 8)
- [x] 3.4 `pane.mark_seen` writes; `herdr.seen=0` in `pane_labels` (verifier 9, 10)
- [x] 3.5 `[experimental] core_label_sync` config, reload, `config-reference.json` entry
- [x] 3.6 Server-path live step: `herdr pane mark-seen` sets the label to the pane's turn (verifier 15, first clause)
- [x] 3.7 Check: `just check`, `just core-live-test`, CI green

As built (PR B). The fact and write types (`CoreAddress`, `Observed`, `CoreFact`, `LabelWrite`) live in
`src/pane/runtime_io/core_labels.rs` so the core backend never imports `app`; the reducer
`src/app/core_labels.rs` and the server side `src/app/core_label_sync.rs` are `#[cfg(unix)]`, like the
event variant `AppEvent::CoreLabels` and every call site, so Windows builds carry none of it.
`CoreClient::subscribe` routes the subscription's channel like an attach stream; the label thread
(`herdr-core-labels`) credits each DATA frame, splits it into lines, and maps `created`/`restarted` and
`labels` with a `record` to `Upsert`. The server recomputes `panes_on_screen` after a pass whose
`sync_core_view` found the view dirty or that drained client events (focus, shown tab, attach). It is one
walk over the attached clients' shown tabs, never per byte or per render. `pane.mark_seen` collects the
unread panes first (`unread_panes`) and raises their labels. Server writes go through
`App::core_label_write`, a recorder in unit tests.

Found by the server live test: a mark read in the seconds before the subscription's first snapshot (the
core starts with the first core pane) wrote nothing. Fixed: the reducer keeps such marks (`marked_early`)
and raises them when the terminal is reported, and the thread retries every 2 s, not 10 s as D11 says.
The same test's live handoff found a second one: `herdr.seen=0` had gone into `pane_labels`, which a
reattach also checks, so after any label write the next server refused the pane's own terminal. The
start label is now added at spawn only (`core_spawn_from`).

Limits: a client-side overlay over the panes still counts as on screen (D7). A label write that core
refuses is logged at debug and not retried; the next look or turn decides again.

## 4. PR C: herdr reads `herdr.seen` and syncs `herdr.pinned`

- [x] 4.1 Read rules of D9 in `CoreLabels::apply`: raise marks seen, lowering marks an idle agent unseen, snapshot raises only (verifier 11, 12)
- [x] 4.2 Pin rules of D10: `herdr_pinned` writes from `pins.rs`, label changes pin or unpin, connect union (verifier 14)
- [x] 4.3 Server-path live steps: a raised label makes a done pane idle; `herdr.pinned=1` pins; `herdr agent unpin` removes the label (verifier 15)
- [x] 4.4 Docs: `docs/next/website/src/content/docs/session-state.mdx`, what is shared with the Herdr apps and the two flags
- [ ] 4.5 Security: ask the security desk for a review of the label reader (D13) before merge
- [ ] 4.6 Check: `just check`, `just core-live-test`, CI green, review answered

As built (PR C). `CoreLabels::apply` takes a `HerdrView` (seen, idle agent, pinned per core address) and
returns an `Outcome` of label writes and `PaneEffect`s (`MarkSeen`, `MarkUnseen`, `Pin`, `Unpin`);
`src/app/core_label_sync.rs` builds the view in one walk over the panes and applies the effects
(`pane.seen`, `pin_agent`, `unpin_agent`). Pins follow D10 through `pin_agreed`, the pin both sides
last agreed on, rather than hooks in `pins.rs`: `CoreLabels::sync_pins` runs after every view change
(`sync_core_view`), so a pin made anywhere in herdr (API, key, menu) reaches the label, and a pane herdr
learns after connect gets the connect rule then. A mark unread needs the label to fall from at least
the turn to below it; a removed label is not a mark unread. Server live steps, after B's mark-seen step:
a second unread turn, `hc label herdr.seen=<turn>` (idle in herdr), `herdr.seen=<turn-1>` (done again),
`herdr.pinned=1` (pinned), `herdr agent unpin` (label removed).

4.5 is open: herdr's review decided the security read happens at ship. 4.6: `just check` and
`just core-live-test` are green locally; CI and the security read are the coordinator's.

Limits: D9's raise race in the mirror gate's window; an unpin made in an app while herdr was not
connected comes back at connect (D10).

## 5. PR D: the `herdr-max` document

- [x] 5.1 `DocSpec` in `spaces_doc.rs`: name and merge as parameters, one thread per document, the 1 MiB refusal (verifier 18)
- [x] 5.2 `src/app/core_extras_doc.rs`: format 1 types and the pure build from `AppState`, wall-clock token expiry (verifier 16, 17)
- [x] 5.3 `[experimental] core_extras_doc` config, shared schedule, `report_metadata` sets the dirty flag, off writes `null`, `config-reference.json` entry (verifier 19)
- [x] 5.4 Live: `doc.get herdr-max` matches `doc.get herdr-app` by id (verifier 20)
- [x] 5.5 Check: `just check`, `just core-live-test`, CI green

As built (PR D). `DocSpec` (`name`, `merge`) has two statics: `HERDR_APP` (marked-spaces merge, the
marking moved to `marked_spaces`) and `HERDR_MAX` (`replace_whole`: no write when core already holds the
value, a warning when another writer's value is replaced). `publish_doc` keeps one writer thread per
document name (`herdr-core-doc-<name>`). A merged value over 1 MiB is not written, with one warning, and
the last written value stays. `src/app/core_extras_doc.rs` builds format 1 from `AppState` with a
`Clock` (one `Instant` and one `SystemTime`) that turns a token's monotonic expiry into `expires_ms`;
panes are listed in layout order, so the value is stable and a repeat is skipped.
`MetadataTokens::entries` exposes key, value and expiry. `publish_core_spaces_if_due` publishes both
documents on the one 500 ms schedule, which `core_extras_doc` alone also arms;
`workspace.report_metadata` sets `core_spaces_dirty`. Live: `core_extras_doc_is_written_whole_and_removed`
(a hand-written value is replaced, `null` removes it) and the server script's join of `herdr-max` with
`herdr-app` (writer session, space id, `pane=address`).

Limits: a token that expires is dropped from the document only at the next publish (readers drop it by
`expires_ms`). Turning `core_spaces_doc` off also clears a pending extras publish until the next change.
Nothing reads the document yet: phase 5 is its first reader.

## 6. Roll-out

- [ ] 6.1 `just beta-local` on m4m after each merged PR; install on both Macs in the same step
- [ ] 6.2 `core_label_sync = true` and `core_extras_doc = true` in ac's config on both Macs (relayed to mac-m4 and mac-m5)
- [ ] 6.3 ac's checks on Herdr Mobile (verifier 21, 22, 23); record the results on #299
- [ ] 6.4 Close #294's "stays done on the phone" limit and #299 after verifier 21 to 23 pass in a running beta
