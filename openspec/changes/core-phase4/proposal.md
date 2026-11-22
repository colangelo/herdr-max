## Why

ac, 2026-11-22: "complete step 3, 4 and 5 today". This change is the rest of step 4 of the herdr-core
migration (`openspec/changes/herdr-core-client/design.md` § 6.4, phase 4 row). Record:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/299. Related: #293 (Herdr Mobile on m4m),
#294 (status mirror follow-ups, including the "stays done on the phone" limit).

Phase 4 started with PR 59 (core-spaces-doc): herdr-max projects its own spaces one way into core's
shared `herdr-app` document, so Herdr Mobile shows them. Two things are left.

1. **The Herdr apps and herdr-max disagree on what ac has seen.** The apps share "seen" and "pinned"
   through terminal labels (herdr-app `docs/labels.md`, commit 8dffe029): `herdr.seen=<turn>` and
   `herdr.pinned=1`. herdr-max neither writes nor reads them. So an agent ac reads in herdr stays
   `done` on the phone (#294), and an agent ac reads on the phone stays `done` in herdr. The phone
   already writes `herdr.seen` on herdr's core terminals.
2. **herdr-max's extras have no shared home.** The `herdr-app` document carries spaces, tabs, names
   and splits only. Pins of spaces, workspace metadata tokens (the asks count), custom names, and pane
   todos live only in this server's memory and `session.json`. Design § 2d and § 6.6 decision 2 put
   them in a core document whose format herdr-max owns, so that a reader on another Mac (phase 5,
   core machines) can show the same world.

A third gap appeared while reading core's turn rules. Core's `turn` goes up only when its root record
leaves `working` (herdr-core `crates/herdr-core/src/status.rs`). The status mirror (core-status-mirror,
PR 58) coalesces records while a pane prints. A short turn can reach core as `done` with no `working`
before it, or not at all when the last record written was already `done`. Core's turn then stays
put. Under the label rule (`done` when `turn > herdr.seen`) that turn shows as `idle` on the phone,
which is a regression once any client writes `herdr.seen`.

## What Changes

- **Every herdr turn reaches core as a turn.** When the mirror coalesced a `working` away, it writes
  `working` and then the newer record in one write, so core's turn goes up once. The mirror also
  re-offers a pane's record when herdr marks the pane seen, so `done` turns into `idle` in core
  without waiting for the next state change (#294 limit).
- **herdr writes `herdr.seen`.** With `[experimental] core_label_sync = true`, the server follows its
  core session's events (`turn` and `labels` only; it still never reads `status`). When ac looks at a
  core pane (a focused client draws it), the server writes `herdr.seen=<turn>` once per turn, only to
  raise it or when it is missing. `herdr pane mark-seen` writes it too. New core panes start with
  `herdr.seen=0`.
- **herdr reads `herdr.seen`.** A label raised to the current turn by another client marks the herdr
  pane seen. A label lowered below the turn (another client's "mark unread") marks an idle agent pane
  unseen.
- **herdr syncs `herdr.pinned`.** Pinning or unpinning an agent in herdr writes the label. A label
  change from another client pins or unpins the agent in herdr. At connect, a pin on either side wins.
- **herdr-max publishes its extras in the core document `herdr-max`.** With
  `[experimental] core_extras_doc = true`, the server writes one document it owns entirely: per space
  the pin order, custom name, and metadata tokens with wall-clock expiry; per tab the custom name; per
  pane the label, agent name and open todo references. Keys are the same ids as the `herdr-app`
  document (`w1`, `w1:t1`, `w1:p1`) plus the core address. The document is a one-way publication:
  herdr-max never reads it back. `session.json` stays the home of every fact.

## Capabilities

### New Capabilities

- `core-turn-fidelity`: core's turn counts herdr's turns, and seen changes reach core's status.
- `core-shared-labels`: `herdr.seen` and `herdr.pinned` shared with the Herdr apps.
- `core-extras-doc`: the `herdr-max` core document.

## Impact

- `src/pane/runtime_io/core_backend/status_mirror.rs` (owed turn), `src/app/core_status_mirror.rs`
  (re-offer on seen changes).
- New `src/app/core_labels.rs` (pure reducer), new `src/pane/runtime_io/core_backend/labels.rs` (the
  subscription and label writes), `src/server/headless/client_views.rs` (the panes on screen),
  `src/app/pins.rs`, `src/app/actions.rs` (seen changes), `src/pane/runtime_io/core_backend/mod.rs`
  (`herdr.seen=0` at spawn).
- New `src/app/core_extras_doc.rs` (document shape), `src/pane/runtime_io/core_backend/spaces_doc.rs`
  (the publisher takes a document name and a merge rule).
- `src/config/model.rs`, `docs/next/website/src/data/config-reference.json`: two new
  `[experimental]` keys, off by default.
- `docs/next/website/src/content/docs/session-state.mdx`: what is shared with the Herdr apps.
- No wire, API or `PROTOCOL_VERSION` change. No `session.json` field. herdr-core stays upstream as
  shipped (hc 0.1.21): nothing is patched there.
