# Design

Sources read for this change (2026-11-22):

- herdr-app `docs/labels.md` at 8dffe029 (the label rules), through `gh api`.
- herdr-core `crates/herdr-core/src/status.rs`, `crates/herdr-core/src/events.rs` and
  `schema/README.md` § Events and § Documents at master, through `gh api`.
- `hc schema` of hc 0.1.21: `terminal.label` (`key=` removes a label), `events.subscribe` (types
  `status`, `turn`, `labels`, `doc`, ...), `Terminal.turn`.
- A live probe on hc 0.1.21 in the throwaway session `p4probe-1122` (stopped afterwards). A program
  printed `working`, `done`, `idle`, then `working` and `done` in one write. `hc events` showed
  `turn` events with `{turn: 1, state: "done"}` and `{turn: 2, state: "done"}`: two records in one
  write end one turn. `hc label t1 herdr.seen=1 herdr.pinned=1` and then `herdr.pinned=` gave
  `labels` events that carry the whole map, and the second removed `herdr.pinned`.

## D1. The `herdr-max` document: who reads it, and why it exists

The premise of design § 2d is "several herdr-max clients and Macs see one world". Tested against
today's code, the premise splits in two.

- **Clients of one herdr-max server already share one world.** Every TUI client, local or remote,
  draws the server's `AppState`. A core document adds nothing for them.
- **Two Macs do not.** m4m and ac-mbm5 each run their own herdr-max server and their own core. A
  reader on ac-mbm5 can read m4m's core documents only through core machines,
  `doc.get {name, on: "m4m"}` (core `schema/README.md` § Documents, the `on` parameter). That is the
  reader phase 5 builds (#300).

So the document's readers are:

1. **Phase 5:** a herdr-max server showing another Mac's world through core machines. It joins that
   Mac's `herdr-app` document (structure) with its `herdr-max` document (extras) by id.
2. **herdr-app mods:** a desktop mod can call `hc doc get herdr-max` to show herdr-max's extras.
3. **People and tools debugging:** `HC_SESSION=herdr hc` shows what herdr-max publishes.

herdr-max itself never reads the document back. `session.json` stays the home of every fact (D2),
as § 6.6 decision 2 and its 2026-11-21 amendment say for the layout.

Alternatives considered:

- **Put the extras inside our marked spaces in `herdr-app`.** The Herdr apps keep unknown space
  fields (`SharedSpace.rest`, core-spaces-doc D1), so this would work today. Rejected: the
  container's format belongs to herdr-app and has no contract, the 1 MiB limit is shared with other
  clients' spaces, and metadata tokens change faster than layout, which would make every app re-read
  its layout. § 6.6 decision 2 asks for a format we own.
- **Labels for everything per pane.** Rejected for todos, names and tokens: labels.md says labels suit
  small values at human speed, and app-wide state belongs in documents. Per-terminal shared state
  that the apps already understand (seen, agent pins) does go in labels (D5).
- **No document until phase 5 needs one.** Simpler (Laziness), but the format would then be designed
  by its first reader alone. This change writes it now and keeps it one-way and versioned, so phase 5
  can ask for a `format: 2` without migrating anything. Open question 1 asks whether to ship PR D now
  or with phase 5.

## D2. What stays in `session.json`

Everything. The document is derived from `AppState` at publish time and is never an input. The
reasons:

- Restore and live handoff already work from `session.json` and the handoff payload. A second source
  would need a precedence rule for every field.
- A core whose state is wiped (`docs.json` gone) must not change herdr-max's state.
- Facts only this server needs (layout ratios, focus, zoom, public id counters, resume plans, launch
  argv, `last_input_at_ms`, former ids, core links) have no reader outside this server.

This change adds no `session.json` field. Seen state still rides the live handoff (`unseen`) as
today. Pin order stays on the workspace and the pane's terminal.

## D3. The document's shape, format 1

Name `herdr-max` (core's name rules: 1 to 32 of `a-z 0-9 - _ .`). Value:

```json
{"format": 1,
 "writer": {"session": "herdr", "version": "0.9.5-ac-beta.210-..."},
 "spaces": [
  {"id": "w1", "pin": 1, "custom_name": "herdr", "tokens": {"asks": {"value": "3", "expires_ms": 1763820000000}},
   "tabs": [{"id": "w1:t1", "custom_name": null}],
   "panes": [{"id": "w1:p1", "terminal": "t7", "label": null, "agent_name": "lead",
              "todos": [{"id": 4, "priority": "high"}]}]}]}
```

- `format`: the version of this shape. Readers must ignore unknown fields. A field's meaning never
  changes inside one format. A change that an old reader would misread needs `format: 2`.
- `writer.session`: the herdr session name, which says which herdr-max server wrote it.
  `writer.version` is the server's version.
- `spaces` is in herdr-max's session order (the manual order). Readers apply their own sort.
- Space `id`, tab `id` and pane `id` are herdr's public ids, the same ids as the `herdr-app`
  document's space, tab and block ids (core-spaces-doc D1). `terminal` is the core address, the same
  as the block's `content.terminal`. A reader joins the two documents on these.
- `pin`: the space's place in the pinned block (1 is first), absent when unpinned. Agent pins are not
  here: they are the `herdr.pinned` label (D5).
- `custom_name`: the name the user set, `null` when herdr shows its automatic label. The shown name is
  in `herdr-app`.
- `tokens`: the workspace metadata tokens (`workspace.report_metadata`) that have not expired.
  `expires_ms` is wall-clock milliseconds since the Unix epoch, converted at publish from the
  token's monotonic expiry; absent for a token without a TTL. Readers drop a token whose
  `expires_ms` has passed. herdr does not republish when a token expires.
- Tab `custom_name`: as for spaces.
- Pane `label` is the name set by `pane.rename`, `agent_name` the agent's name (`agent rename` or the
  fallback). Both are `null` when unset.
- `todos`: references to the pane's open todos, `{id, priority}`, at most the pane's limit of 50.
  The text stays behind herdr's API (`herdr todo list`). Done todos are left out.
- Panes without a core terminal are left out, as in `herdr-app` (core-spaces-doc D2).
- Seen state is not in the document. It is the `herdr.seen` label (D5).

Size: 51 panes with 50 todo references each is about 80 KB, well under core's 1 MiB. The publisher
still refuses to write a value over 1 MiB, logs a warning once, and keeps the last written value.

## D4. The publisher

`core_backend/spaces_doc.rs` today hard-codes the name `herdr-app` and the merge rule of
core-spaces-doc D6. It takes both as parameters instead:

```rust
pub(super) struct DocSpec {
    name: &'static str,
    /// The value to write given core's current value, or None when nothing changes.
    merge: fn(current: &Value, ours: &Value) -> Result<Option<Value>, Undecodable>,
}
```

- `herdr-app` keeps its merge (ours replace the marked spaces).
- `herdr-max` replaces the whole value. Its merge returns `None` when `current == ours`. A current
  value with a different `writer.session` is overwritten with a warning: one herdr session writes
  one core session (core-backend D2), so another writer is a stale value.
- Each document gets its own thread and its own `DocPublisher`: latest offer wins, compare-and-set,
  three retries on conflict, 10 s retry when core is unavailable, never starts a core
  (core-spaces-doc D5).

The server publishes both documents on the existing schedule: one 500 ms debounce
(`core_spaces_deadline`), set by every change the session saves. Metadata tokens are not saved in
the session, so `workspace.report_metadata` also sets the dirty flag. Each publisher skips a value
equal to the last one written, so sharing the trigger costs no extra write. The flag
`[experimental] core_extras_doc` gates the `herdr-max` document alone. Turning it off writes `null`.

## D5. Per-terminal facts go in labels

labels.md defines two shared per-terminal facts. herdr-max maps them onto its own:

| Label | herdr-max fact | Where it lives in herdr-max |
|---|---|---|
| `herdr.seen=<turn>` | pane read state (`PaneState.seen`, `done` = idle and not seen) | `AppState`, carried by the live handoff |
| `herdr.pinned=1` | agent pin (`TerminalState.pin_order`) | `session.json` |

Labels live on the terminal, so they reach every client, local or remote (`mac/t7`), with no
document. Space pins have no terminal, so they go in the document (D3).

## D6. Core's turn must count herdr's turns

The label rule compares `turn` with `herdr.seen`, so core's turn must go up once for every turn
herdr shows. herdr's state reaches core only through the mirror, and core-status-mirror D3 coalesces
records while the pane prints. Two cases lose a turn:

- The agent starts and finishes while output never pauses for 100 ms. The pending `working` is
  replaced by `done` before the gate opens. Core goes from `idle` or `done` straight to `done`.
- The last record written was `done` (the previous turn is still unread). The new turn's `done` equals
  it, so `offer` drops it as redundant, and nothing is written.

The fix lives in `StatusMirror` and is one fact, `owed_turn: bool`:

- `offer(Working)` while the last written record is not `Working` sets `owed_turn`.
- `offer(r)` for any other record keeps `r` pending even when it equals the last written record, if
  `owed_turn` is set.
- Writing `Working` clears `owed_turn`. Writing any other record `r` with `owed_turn` set writes
  `encode(Working)` followed by `encode(r)` in the one `write` call, then clears it. Both records
  together stay under 256 bytes.

The probe above shows two records in one write end one turn in core.

Seen changes reach core too. herdr turns `done` into `idle` by marking the pane seen, with no state
update, so the mirror kept `done` (core-status-mirror D6, the #294 limit). The server now re-offers the
records of every pane after a change to what is on screen or to read state (D7's view trigger). The
mirror drops records equal to the last written one, so an unchanged pane costs one comparison.

Agents that report OSC 7501 themselves (Claude Code, Codex, pi; design § 4b) already move core's turn.
For them a coalesced turn now ends two turns in core, one from the agent and one from the owed
`working`. Labels compare turns and never count them, so a double step changes nothing a client
shows. Core's status may show `working` for the instant between the two records.

Alternatives considered:

- **Owe the turn only before `done`.** Smaller, but then `working` to `blocked` and `working` to
  `idle` (seen) can still skip a turn, and the rule would need a reason per word. Rejected.
- **Decide in the server from core's turn events** (owe a turn only when core's turn did not move
  since herdr's turn began). This removes the double step for native reporters, but couples the mirror
  to the label subscription and gives two rules depending on a flag. Rejected for now. Revisit if the
  instant of `working` shows on the phone.

## D7. What "looking" means in herdr-max

labels.md: a person is looking at an agent when the window is in the foreground and the agent is
drawn on screen. A pane covered by another terminal does not count. The server has the facts it needs
(`src/server/headless/client_views.rs`):

```rust
/// Core addresses of the panes some person is looking at now.
fn panes_on_screen(&self) -> HashSet<CoreAddress>
```

A pane is on screen when an attached shell client has `outer_terminal_focus == Some(true)` (the
outer terminal reported focus-in), the client's shown tab (`shell_target_for_client`) holds the pane,
and the tab is not zoomed on another pane. `None` (a terminal that never reports focus) does not
count. herdr's own read marking treats `None` as focused (`active_tab_suppresses_notifications`), but
labels clear the mark on every client, so the label rule is the stricter one.

The server recomputes the set after a view change: a client's focus, a client's shown tab, a zoom,
a client attaching or leaving, and every change the session saves (which covers tab switches and
layout changes). This is the same `view_dirty` flag that triggers the mirror re-offer (D6). It
is consumed once per server loop pass, so a burst costs one pass. The work is one walk over the
attached clients' tabs, at human speed. It runs in no render, view or PTY loop.

Limits: a client-side overlay that covers the panes (a full-screen panel) is not known to the server,
so its panes still count as on screen. A Mac whose display sleeps while the terminal keeps focus
still counts. The once-per-turn rule (D8) bounds what such a screen can clear.

## D8. Writing `herdr.seen`

The reducer (D11) keeps, per core terminal, `decided: Option<u64>`: the turn it last decided on while
the pane stayed on screen.

- When a pane is on screen and `decided != Some(turn)`: set `decided = Some(turn)`. If the label is
  missing or below `turn`, write `herdr.seen=<turn>`.
- When a pane leaves the screen: `decided = None`. Looking again later decides again.
- A `turn` event for a pane on screen decides on the new turn (labels.md: decide on the `turn`
  event, not on `status`).
- `herdr pane mark-seen` (and its menus) writes `herdr.seen=<turn>` for every core pane it marked
  read, whether on screen or not. It is a person's explicit read.
- A new core terminal herdr starts carries `herdr.seen=0` in its `terminal.run` labels
  (`pane_labels`).

Because a decision happens once per turn per stretch on screen, another client's "mark unread" is not
overruled by a screen that stays on.

## D9. Reading `herdr.seen`

A `labels` event carries the whole map. The reducer compares the new `herdr.seen` with the one it
held:

- **Raised to the turn or above** (`new >= turn`, and `old` was missing or below `turn`): the herdr
  pane is marked seen. It stops being `done` in herdr's sidebar, agent list and API.
- **Lowered below the turn** (`new < turn <= old`): another client marked it unread. If the herdr
  pane is an agent in state `Idle` and seen, it is marked unseen (`done`).
- Anything else changes nothing in herdr.

herdr's own writes come back as `labels` events. They are raises to a turn that herdr already shows as
read, so applying them is a no-op. No echo tracking is needed.

At connect (the subscription's snapshot), only the raise rule applies: a pane whose label is at or
above its turn is marked seen. A turn event alone never marks a pane unseen. herdr's own completion
rule (hooks and detection) still decides when a pane becomes `done` (M3: herdr's state authority is
unchanged).

A race remains. If another client raises the label in the moment between herdr's completion and core's
turn event for it (the mirror's 100 ms quiet gate plus core's read), herdr reads the raise as covering
the new turn and marks it seen early. When core's turn moves, the other client shows `done` again
until someone looks. This needs a person to open the pane on another client in that window.

## D10. `herdr.pinned`

- herdr pins an agent (`agent.pin`, the key, the menu): write `herdr.pinned=1`. Unpin: write
  `herdr.pinned=` (removes it).
- A `labels` event where `herdr.pinned` became non-empty pins the agent at the end of herdr's pin
  order. Where it became empty or missing, herdr unpins it.
- At connect: a pane pinned in herdr but not labelled gets the label. A pane labelled but not pinned in
  herdr is pinned in herdr.

The connect rule means an unpin on another client while herdr was not connected comes back. A
three-way merge would need the last agreed value saved per pane in `session.json`. Rejected for that
cost; live events cover the normal case.

## D11. The reducer and the subscription

The rules of D8 to D10 are one pure reducer in `src/app/core_labels.rs`, tested without a core:

```rust
pub(crate) struct CoreLabels { terminals: HashMap<CoreAddress, Shared> }
struct Shared { turn: u64, seen: Option<u64>, pinned: bool, decided: Option<u64> }

pub(crate) enum CoreFact {
    Snapshot(Vec<(CoreAddress, Observed)>),
    Turn { address: CoreAddress, turn: u64 },
    Labels { address: CoreAddress, seen: Option<u64>, pinned: bool },
    Gone { address: CoreAddress },
    Disconnected,
}
pub(crate) struct Observed { turn: u64, seen: Option<u64>, pinned: bool }
pub(crate) enum LabelWrite { Seen { address: CoreAddress, turn: u64 }, Pinned { address: CoreAddress, pinned: bool } }
pub(crate) enum PaneEffect { MarkSeen(CoreAddress), MarkUnseen(CoreAddress), Pin(CoreAddress), Unpin(CoreAddress) }

impl CoreLabels {
    pub(crate) fn apply(&mut self, fact: CoreFact, herdr: &impl HerdrView) -> (Vec<LabelWrite>, Vec<PaneEffect>);
    pub(crate) fn look(&mut self, on_screen: &HashSet<CoreAddress>) -> Vec<LabelWrite>;
    pub(crate) fn herdr_pinned(&mut self, address: &CoreAddress, pinned: bool) -> Option<LabelWrite>;
    pub(crate) fn herdr_marked_seen(&mut self, address: &CoreAddress) -> Option<LabelWrite>;
}
```

`HerdrView` answers "is this pane seen, idle, an agent, pinned" for the address, so the reducer never
holds herdr's state. A label value that is not a decimal `u64` reads as missing for `herdr.seen`.

The I/O lives in `src/pane/runtime_io/core_backend/labels.rs`:

- It uses the server's one core connection (`existing_core_connector`, which never starts a core) and
  calls `events.subscribe {labels: {"herdr.session": <label>}}`, so it sees only this herdr session's
  terminals. In framed mode the events arrive as JSON lines on the subscription's channel (core
  `events.rs`, `Sink::Framed`). The client's reader routes that channel the way it routes attach
  streams.
- It parses `turn`, `labels`, `removed`, `finished`, `missed` (a fresh snapshot) and `heartbeat`. It
  drops `status` events unparsed: herdr still never reads core's status (security review M3).
- Facts go to the server loop as `AppEvent::CoreLabels(CoreFact)`. Writes come back from the server
  over an `mpsc` channel and become `terminal.label` calls, latest per address and label wins, one call
  in flight, 5 s timeout.
- On a lost connection it sends `Disconnected`, retries every 10 s, and starts from a fresh snapshot
  (core's `seq` restarts per core process).

The server maps an address to its pane through the pane runtime's core link, as
`publish_core_spaces_if_due` does (`primary_address`). Panes in another core session are not
followed.

## D12. Flags

- `[experimental] core_label_sync`, default false, reload applies it. On: subscribe, write and read.
  Off: unsubscribe and write nothing. Labels already written stay; they are valid for other clients.
- `[experimental] core_extras_doc`, default false, reload applies it (D4).
- Neither needs the other. `core_label_sync` without `core_status_mirror` works but core's turn then
  moves only for agents that report OSC 7501 themselves. ac's config turns all three on.

## D13. Security

- New input from core to herdr state: any process that can reach this user's core socket can write
  `herdr.seen` and `herdr.pinned`, and so mark herdr panes read, unread, pinned or unpinned. That is the
  feature, and the socket is already the same user's (core `docs/security.md`). No label can change an
  agent's state, start, stop or type into anything.
- Label values are parsed, never interpolated: `herdr.seen` as a decimal `u64`, `herdr.pinned` as
  empty or not.
- The document publishes custom names, pane labels, agent names, token values and todo ids to the
  same user's core. A remote reader sees them only through core machines, which already reach every
  terminal's screen.
- herdr still ignores core's `status` (M3). The V8 test stays.

## Limits

- D7: client-side overlays and sleeping displays count as on screen.
- D9: the raise race in the mirror's gate window.
- D10: an unpin made elsewhere while herdr was not connected comes back at connect.
- D6: native 7501 reporters can step core's turn twice for one turn.
- D3: a reader that ignores `expires_ms` shows an expired token until herdr's next publish.
- Panes on the PTY (no core terminal) get no labels and are not in the document.

## Verifier table (required tests)

The answer key for rows 1 to 4 is herdr-core itself: its turn rule (`status.rs` `end_turn`) as
`hc get --json` and `hc events` report it. For rows 5 to 15 it is herdr-app `docs/labels.md` at
8dffe029. For rows 16 to 20 it is D3 of this design. Unit rows run in `just check`. Live rows run
with `just core-live-test` against a throwaway hc 0.1.21 session. Rows 21 to 23 are ac's checks.

| # | Proves | Where | Pass when |
|---|---|---|---|
| 1 | coalesced `working` is owed | unit, `core_backend/status_mirror.rs` | offers `working`, `done` before the gate opens: the due write is `working` then `done`; after a written `done`, offers `working`, `done` write `working` then `done`, not nothing |
| 2 | no owed turn without `working` | unit, `status_mirror.rs` | `idle` then `done` writes `done` alone; a written `working` clears the debt |
| 3 | live: one turn per herdr turn | live, `src/pane/core_backend_live_tests.rs` | a core pane whose mirror gets `working`, `done` within 50 ms of output: `hc get` turn rose by exactly 1 |
| 4 | seen change reaches core | unit, `src/app/core_status_mirror.rs`; live, same test | marking a `done` pane seen re-offers `idle`; live `hc get` shows `status.state=idle` with no herdr state change |
| 5 | write on looking, raise only | unit, `src/app/core_labels.rs` | on screen at turn 3 with label 1 or missing: one `Seen{3}`; with label 3 or 5: none |
| 6 | once per turn on screen | unit, `core_labels.rs` | on screen, label lowered to 2 at turn 3 by another client: no write until the pane leaves and returns, or turn 4 arrives |
| 7 | decide on the turn event | unit, `core_labels.rs` | a pane on screen gets `Turn{4}`: one `Seen{4}` |
| 8 | what counts as on screen | unit, `src/server/headless` tests | focused client on the tab: its panes; zoomed tab: only the zoomed pane; client focus `Some(false)` or `None`: none; two clients on two tabs: both tabs |
| 9 | mark-seen writes | unit, `src/app/api/panes.rs` | `pane.mark_seen` on two done core panes yields two `Seen` writes at their turns |
| 10 | spawn label | unit, `core_backend/mod.rs` | `terminal.run` labels hold `herdr.seen=0` |
| 11 | read: raise marks seen | unit, `core_labels.rs` | a done pane at turn 3 gets label 3: `MarkSeen`; label 2: nothing |
| 12 | read: mark unread | unit, `core_labels.rs` | seen idle agent at turn 3, label 3 then 2: `MarkUnseen`; same for a working agent or a non-agent: nothing |
| 13 | status stays unread | unit, `core_backend/labels.rs` | a `status` event yields no `CoreFact`; V8 (`core_v8_printed_status_is_not_an_agent_report`) passes unchanged |
| 14 | pinned both ways | unit, `core_labels.rs` and `src/app/pins.rs` | herdr pin gives `Pinned{true}`, unpin `Pinned{false}`; label set gives `Pin`, label removed gives `Unpin`; snapshot union as D10 |
| 15 | live: labels round trip through a server | `scripts/core_live_test.sh` | with `core_label_sync = true`: `herdr pane mark-seen` sets `herdr.seen` to the pane's turn; `hc label <t> herdr.seen=<turn>` on a done pane makes `herdr agent list` show it idle; `hc label <t> herdr.pinned=1` makes it pinned; `herdr agent unpin` removes the label |
| 16 | document shape | unit, `src/app/core_extras_doc.rs` | a pinned, custom-named space with a token, a renamed tab, a pane with a label, agent name and two open todos and one done: fields as D3; done todo absent; ids equal `spaces_document`'s ids |
| 17 | token expiry in wall time | unit, `core_extras_doc.rs` | a token with a 60 s TTL publishes `expires_ms` within 1 s of now + 60 000; an expired token is absent |
| 18 | whole-value owner | unit, `core_backend/spaces_doc.rs` | `herdr-max` merge: equal value gives no `set`; a foreign value is replaced; conflict retries; a value over 1 MiB makes no `set` |
| 19 | config and schedule | unit, `src/config/model.rs`, `core_extras_doc.rs` | both keys default false and parse true; a metadata report sets the dirty flag; flag off publishes `null` |
| 20 | live: the document in core | live, `core_backend_live_tests.rs` and `scripts/core_live_test.sh` | `doc.get herdr-max` returns format 1 with the server's spaces; its pane ids and terminals match `doc.get herdr-app` |
| 21 | phone follows herdr | ac, Herdr Mobile on m4m | an agent finishes, ac reads it in herdr, the phone's Inbox stops showing it `done` |
| 22 | herdr follows the phone | ac, Herdr Mobile on m4m | an agent finishes, ac opens it on the phone, herdr's sidebar stops showing it `done` |
| 23 | pins match | ac, Herdr Mobile on m4m | an agent pinned in herdr is pinned on the phone, and the reverse |
