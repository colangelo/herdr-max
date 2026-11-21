# Design

## D1. The document the apps read

The source is `github.com/herdrdev/herdr-app`: `src/mirror.rs` (`Document`,
`SharedSpace`, `SharedTab`), `src/layout.rs` (`Node`, `Kind`, `Content`, `Size`) and
`src/mirror_sync.rs` (`doc.get`/`doc.set` on name `herdr-app`). herdr-core's
`schema/README.md` § Documents defines the calls. The shape herdr writes:

```json
{"spaces": [{"id": "w1", "name": "herdr", "folder": "/Users/ac/dev/herdr", "herdr_max": 1,
  "tabs": [{"id": "w1:t1", "name": "1", "root":
    {"id": "w1:t1", "type": "row", "children": [
      {"id": "w1:p1", "type": "block", "content": {"terminal": "t7"}, "size": {"weight": 0.5}},
      {"id": "w1:t1:s2", "type": "column", "size": {"weight": 0.5}, "children": [
        {"id": "w1:p2", "type": "block", "content": {"terminal": "t8"}, "size": {"weight": 0.6}},
        {"id": "w1:p3", "type": "block", "content": {"terminal": "t9"}, "size": {"weight": 0.4}}]}]}}]}]}
```

- Space: `id` is the public workspace id (`w1`). `name` is the name herdr shows (the
  custom name, else the automatic label). `folder` is the cwd of the first tab's root
  pane, else the workspace's identity cwd. `herdr_max: 1` marks the space as herdr-max's
  own (D6). The writer adds it. The apps keep unknown space fields across their own
  writes (`SharedSpace.rest`, `#[serde(flatten)]` in `mirror.rs`), so the mark survives
  a phone or Mac app write.
- Tab: `id` is the public tab id (`w1:t1`). `name` is the custom name, else the tab's
  number, as herdr shows it.
- `root`: a container whose `id` is the tab id, as the Mac app writes it
  (`Tab::with_pane`). A herdr split with direction horizontal (panes side by side) is a
  `row`, vertical is a `column`. The first child has weight `ratio` and the second has
  `1 - ratio`. A tab that is one pane is a `row` with one block.
- Block: `id` is the public pane id (`w1:p1`). `content.terminal` is the pane's core
  address (`t7`, the `address` field of `terminal.list`).
- Ids are herdr's public ids, so they stay stable across publishes and the app keeps its
  focus and zoom choices per tab. No id contains `/`, which the app uses to qualify
  remote machines.

## D2. Panes that are not in core

A pane without a core terminal (PTY, `pane_jobs`, a pane being respawned) has no
address the phone could attach to. It is left out. Its split is replaced by the other
side, which keeps the slot's size. A tab with no core pane has an empty `row` root. The
app reads an empty root as a tab with no terminals (`empty_root`).

## D3. Types

`src/app/core_spaces_doc.rs` holds `SpacesDocument`, `DocSpace`, `DocTab` and `DocNode`.
They are `Serialize`-only structs, and their field names and tags match the app's.
`DocNode` has an `id`, an optional `size`, and a `DocNodeKind` enum
(`Block { content }`, `Row { children }`, `Column { children }`) tagged with `type`.
`spaces_document(workspaces, terminals, core_address)` is pure. Its last argument is a
closure from terminal id to core address, so tests need no runtime.

## D4. When the server publishes

- `AppState::mark_session_dirty` and `mark_session_dirty_urgent`, and
  `App::schedule_session_save` and `schedule_session_save_now`, also set
  `AppState::core_spaces_dirty`. Every layout change (workspace create, close, rename,
  move; tab create, close, rename; split, close, swap, resize, respawn) already passes
  through one of them, because it changes the saved session.
- The server loop calls `App::sync_core_spaces_schedule` next to
  `sync_session_save_schedule`. With the flag on, a dirty flag sets
  `core_spaces_deadline` to now + 500 ms when no deadline is set. Bursts therefore give
  one publish per 500 ms. With the flag off, the dirty flag is dropped.
- When the deadline is due, `handle_scheduled_tasks_headless` calls
  `App::publish_core_spaces`. It builds the document and hands it to the publisher.
- `App::new` and `App::new_from_handoff` set the deadline to now when the flag is on.
- Reload uses `App::set_core_spaces_doc(enabled)`. Turning the flag on sets the
  deadline to now. Turning it off clears the deadline and hands the publisher `null`.

The build walks workspaces, tabs and panes once per publish. That is at most twice a
second and only on human-speed changes. It runs in no render, view or PTY loop.

## D5. The publisher

`src/pane/runtime_io/core_backend/spaces_doc.rs`. `crate::pane::publish_core_spaces`
sends herdr-max's spaces, a `Vec<serde_json::Value>`, over an `mpsc` channel. An empty
list withdraws them. The first call
starts one thread, `herdr-core-spaces`, which lives as long as the process. The server
main loop never waits on core.

The thread owns a `DocPublisher<S: DocStore>`:

```rust
struct Doc { value: Value, version: u64 }
trait DocStore {
    fn get(&mut self) -> Result<Doc, StoreError>;               // current value and version
    fn set(&mut self, value: &Value, version: u64) -> Result<u64, StoreError>;
}
enum StoreError { Conflict, Unavailable(String) }
struct DocPublisher<S> { store: S, written: Option<Vec<Value>>, pending: Option<Vec<Value>> }
fn merge(current: &Value, ours: &[Value]) -> Result<Option<Value>, Undecodable>;
```

- `offer(spaces)`: marks each space with `herdr_max: 1`. The latest offer wins. An offer
  equal to `written` (our spaces as last merged) clears `pending`.
- `flush()`: with nothing pending it does nothing. Otherwise it reads the document's
  value and version, merges our spaces into the value (D6), and calls `set` with that
  version. A merge that changes nothing writes nothing. Success records `written`. On
  `Conflict` it reads and merges again, at most 3 times. On `Unavailable`, on an
  undecodable value, or when the retries run out, it keeps `pending`. The thread then
  waits 10 s or for the next offer, whichever comes first, and flushes again.
- The thread blocks on `recv` when nothing is pending. After each message it drains the
  channel, so one write covers a burst. Only one write is ever in flight.

`CoreDocStore` is the real store. It uses `existing_core_connector()`, the server's one
core connection, which never starts a core, and calls `doc.get` and `doc.set` on name
`herdr-app` with a 5 s timeout. Core's `conflict` error kind maps to `Conflict`, and
every other error maps to `Unavailable`. It logs the first unavailability as a warning
and later ones at debug level.

## D6. herdr-max owns only its marked spaces

Other clients write `herdr-app` too. The phone app writes it (`{"spaces":[]}` at version 4
was seen in the default core session), and a Herdr Mac app on the same machine would. So
herdr-max owns only the spaces it marked, and `merge(current, ours)` is the whole rule:

- A space with a `herdr_max` field is ours. Every other space is foreign. herdr never
  changes, moves relative to each other, or removes a foreign space, and keeps the
  document's other top-level fields.
- Ours replace the marked spaces as one block, at the place of the first marked space.
  With no marked space yet, ours go after the foreign ones. This follows the Herdr app's
  own `Document::replace`, which puts a machine's spaces where that machine's first space
  was. A user who moved herdr-max's spaces in an app keeps that place.
- An empty `ours` (the flag turned off) removes the marked spaces. The document becomes
  `null` only when no foreign space and no other top-level field remains. A document
  with no marked space and an empty `ours` is left alone, so a foreign-only document is
  never written.
- A merge that would not change the spaces writes nothing.
- The value must be `null` or an object whose `spaces` is absent or a list of objects.
  Anything else is undecodable. herdr does not overwrite it, logs a warning once, and
  tries again after 10 s.

The merge runs on the value just read, and `doc.set` is a compare-and-set on that
version. A foreign write between the read and the write makes core answer `conflict`, and
the writer reads and merges again. A foreign write can therefore never be lost.

One way still holds for ours: herdr does not subscribe to doc events, and its layout is
the source of its own spaces (#293 (b)). An app's edit to one of herdr's spaces stays
until herdr's next publish, which replaces it.

## D7. Server tests use no core

`App` holds `core_spaces_publish: fn(serde_json::Value)`. In builds it is
`crate::pane::publish_core_spaces`. In tests it records the value in a thread-local, so
no unit test can reach a user's core (`server_hangup_probe` uses the same pattern).

## Limits

- An app's edit to one of herdr's spaces is replaced only at herdr's next layout change,
  not at once (D6). An app that removes herdr's spaces sees them return then too.
- The mark is the field's presence. A foreign client that copies a marked space keeps
  the mark, so herdr takes that copy for its own and replaces it.
- A pane's cwd change alone does not republish the space's `folder`. The next layout
  change carries it.
- The document holds only core panes (D2). On m4m every pane is a core pane since
  2026-11-21.
- The phone app's decoder is not public. Its shape is taken to be the Mac app's
  (`mirror.rs`), which writes the same document. Verifier row 12 is ac's check on the
  phone.

## Verifier table (required tests)

The answer key for 1 to 4 is the Herdr app's own serde shapes. A test-only copy of
`Document`, `SharedSpace`, `SharedTab`, `Node`, `Kind`, `Content` and `Size` from
`herdr-app` `src/mirror.rs` and `src/layout.rs` decodes what herdr writes, and that
copy also decodes the app's own fixture from
`unknown_space_and_tab_fields_survive_a_read_and_write`. Rows 1 to 9 and 14 to 18 are
unit tests. Rows 10, 11 and 13 run with `just core-live-test` against a throwaway `hc`
0.1.21. Row 12 is ac's own check on the phone.

| # | Proves | Where | Pass when |
|---|---|---|---|
| 1 | the copy matches the app | unit, `src/app/core_spaces_doc.rs` | the app's fixture JSON decodes with the copied shapes |
| 2 | spaces, tabs and names | unit, `core_spaces_doc.rs` | two workspaces with custom and automatic names, two tabs: ids `w…`/`w…:t…`, names and folder as D1; decodes with the copy |
| 3 | split tree | unit, `core_spaces_doc.rs` | horizontal → `row`, vertical → `column`, weights `ratio`/`1-ratio`, root id is the tab id, leaves carry the core address |
| 4 | non-core panes left out | unit, `core_spaces_doc.rs` | a split with one PTY side collapses to the core side, which keeps the slot's size; an all-PTY tab has an empty `row` root; it still decodes |
| 5 | CAS write and mark | unit, `core_backend/spaces_doc.rs` | into an empty document, one get then one set at version 0; each space carries `herdr_max: 1` |
| 6 | conflict re-merge | unit, `spaces_doc.rs` | a foreign write between get and set gives `Conflict`; the retry merges into the foreign value and keeps its new space; three conflicts in a row keep ours pending |
| 7 | unavailable | unit, `spaces_doc.rs` | `Unavailable` keeps ours pending; the next flush gets and sets |
| 8 | latest wins, no repeat | unit, `spaces_doc.rs` | three offers before a flush write only the last; offering it again writes nothing |
| 9 | config and server schedule | unit, `src/config/model.rs` and `core_spaces_doc.rs` | default false, `core_spaces_doc = true` parses; flag off: a dirty flag sets no deadline and nothing is published; on: dirty sets a deadline 500 ms out, due publishes the document; turning off publishes `null` |
| 10 | live: merge beside a foreign space | live, `src/pane/core_backend_live_tests.rs` | core holds a foreign space; after `publish_core_spaces` with a space whose leaf is a real core pane's address, `doc.get herdr-app` returns the foreign space unchanged, then ours with `herdr_max: 1` |
| 11 | live: foreign write and withdraw | live, same test | after a foreign `doc.set` moves ours first and adds a second foreign space, the next publish replaces ours in place and keeps both foreign spaces; publishing no spaces leaves only the two foreign ones |
| 12 | the phone shows spaces | ac, Herdr Mobile on m4m, beta with the flag on | ac's workspaces appear as spaces with their tabs and terminals (not run by the builder) |
| 13 | live: the server path | `scripts/core_live_test.sh` | with the flag on, the document is one marked space whose only leaf is split core pane B (`B=<address>`), not PTY pane A; after a foreign write of space `s9` alone, a restarted server's document is `s9` then ours; reload with the flag off leaves `s9` alone |
| 14 | foreign kept in place, ours replaced | unit, `spaces_doc.rs` | `[f1, ours, f2, ours]` with ours `[w1, w2]` gives `[f1, w1, w2, f2]`; `f1` and `f2` keep every field |
| 15 | ours after foreign when new | unit, `spaces_doc.rs` | `{spaces: [f1], other}` gives `[f1, w1]` and keeps `other` |
| 16 | off removes only ours | unit, `spaces_doc.rs` | `[ours, f1, ours]` with no spaces gives `[f1]`; `[ours]` alone gives `null` |
| 17 | foreign-only left alone | unit, `spaces_doc.rs` | `null`, `{spaces: []}` and `{spaces: [f1]}` with no spaces of ours make no `set` |
| 18 | undecodable untouched | unit, `spaces_doc.rs` | a string, an array, `spaces` as an object, and a non-object space: no `set`, `Retry`; once the value decodes, the next flush lands |
