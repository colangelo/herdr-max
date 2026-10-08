# Design

## What is already load-bearing and must survive

`AppState` owns overlay geometry and both the renderer and the mouse hit-test
read it — `notification_center_list_window` (`src/app/input/mouse.rs:1567`) is
commented *"Shared by render and mouse hit-testing so they agree"*. That kills
the classic TUI bug where the click map drifts from the drawing. The kit does not
move geometry into the renderer; it factors the *arithmetic* out while leaving
the accessors exactly where they are.

The second invariant is the project's own: `compute_view()` mutates, `render()`
does not. Nothing in the kit holds mutable widget state, which is why none of the
component frameworks below fit.

## The four primitives

### `AnchoredPanel`

```
AnchoredPanelSpec {
    anchor: Rect,            // tab bar, pane rect, or screen
    screen: Rect,
    content_width: u16,      // measured by the caller from its own rows
    width: (u16, u16),       // clamp bounds; both panels use (30, 60)
    rows: u16,
    max_rows: u16,
    footer_rows: u16,
    horizontal: Edge,        // Right today for both
    vertical: Anchor,        // BelowAnchor | AboveAnchor | InsideTop
}
-> PanelGeometry { outer, inner, list, footer_row: Option<Rect> }
```

Everything except `content_width` is either constant or already computed at both
call sites. `content_width` stays with the caller because measuring rows is the
one genuinely per-panel thing — the notification center measures title plus
context plus an age column, the todo panel measures text plus a link chip.

`vertical: AboveAnchor` covers the notification center's bottom-right mode, which
opens above the floating indicator; `BelowAnchor` covers its top-right mode and
the todo panel hanging off a pane's top border.

### `ButtonRow<B>`

Built from `&[(B, Option<&str>, &str)]` — button, key hint, label — reusing the
existing `action_button_text` / `action_button_width` / `action_button_row_rects`
in `src/ui/widgets.rs`, which already do the per-button maths. It adds what the
two panels each hand-rolled around them: which buttons fit at this width, the
resulting rects, `hit(col, row) -> Option<B>`, and `row_y()`.

The graceful-degradation rule both panels implement by hand — drop optional
buttons until the row fits, never drop the close button — becomes a property of
the row: buttons carry a priority and the row drops from the bottom up.

### `ListCursor`

`MenuListState` and `SelectionListState` merge. The merged type adds the
windowing that overlays currently re-derive:

```
ListCursor { selected: usize }
  move_by(delta: isize, len: usize)
  select(idx), hover(Option<idx>)
  window(list: Rect, len: usize) -> (start: usize, visible: usize)
  row_at(list: Rect, col, row, len) -> Option<usize>
```

`window` is the "keep the selection visible, nearest-edge reveal" rule already
implemented separately in the navigator, the notification center, and the todo
panel. `row_at` is the inverse and is what makes mouse and render agree by
construction rather than by two functions that happen to match.

Deliberately *not* included: the sidebar's bubble motion (`src/ui/list_motion.rs`).
It is a display-order transform over a settled list, it composes with a cursor
rather than living in one, and folding it in would drag animation state into a
type the modals also use.

### `TextField`

Introduced by `todo-editing-keyboard` for todo composition and adopted here by
the rename modals, worktree create, and the navigator / keybind-help search
boxes. Each of those is its own append-only implementation today
(`handle_rename_edit_key`, `insert_navigator_search_text`,
`insert_keybind_help_query_text`, …), and each is missing a cursor for the same
reason. Adoption is mechanical once the primitive exists, which is why this
change follows rather than leads.

## The overlay descriptor

Today: `Mode` (24 variants) plus ten-plus parallel `Option<XState>` fields on
`AppState`, paired by convention. Proposed:

```
enum Overlay {
    Settings(SettingsState),
    NotificationCenter(NotificationCenterState),
    PaneTodos(PaneTodoPanelState),
    PaneTodoEdit(PaneTodoEditState),
    ContextMenu(ContextMenuState),
    ...
}
AppState { overlay: Option<Overlay>, ... }
```

with `Overlay::mode()`, `wants_ascii_input()`, `honors_key_repeat()`, and
`help_entries()` on the enum. Three consequences:

1. `mode == PaneTodos` with `pane_todos == None` stops being representable, and
   the `?`-guards that defend against it stop being load-bearing.
2. `Mode::wants_ascii_input` and `Mode::honors_key_repeat` — both documented as
   hand-maintained allowlists that a new mode can silently miss — derive from the
   variant instead.
3. `AGENTS.md`'s "new keybindings must be discoverable in the help panel" becomes
   a method that must be implemented rather than a rule humans must remember. A
   guard test asserts every variant contributes at least one help entry, in the
   spirit of the existing test that fails when a keybinding action is present in
   the serialize config but absent from the deserialize overlay.

`Mode` itself stays, because non-overlay modes (`Terminal`, `Prefix`, `Copy`,
`Navigate`, `Resize`) are genuinely modes and not overlays, and because the
input dispatch is keyed on it. The enum supplies the mode rather than replacing
it.

This is the piece with real risk: `Mode` and `AppState` are what upstream edits
most, so it is the largest rebase cost in the change, and it touches state
identity. It is last for that reason, and it is the only group requiring the
`AGENTS.md` refactor-risk protocol.

## Verification strategy

The kit's promise is *no visible change*, which is exactly what a snapshot test
proves and what unit tests do not. `todo_panel.rs`'s tests already render into a
ratatui `TestBackend` buffer and read rows back with a local `row_text(buffer,
rect)` helper; promoting that into a shared test utility gives every overlay a
before/after comparison at a fixed terminal size.

Order of operations per group: write the snapshot test against current
behaviour, land it green, then refactor underneath it. A group whose snapshot
changes has either found a real bug or broken something, and either way the diff
says which.

`insta` was considered for snapshots and rejected: a new dependency for what a
`Vec<String>` comparison already does, and inline expected output keeps the test
readable next to the code it covers.

## Alternatives considered

**`tui-realm`.** A real component framework for ratatui: props, component state,
message passing, focus management. It wants to own the event loop and hold
mutable per-component state — a head-on collision with "state is separated from
runtime, render is pure", and with the runtime/client split the project is
migrating toward. Adopting it is a rewrite of `src/app/input` and `src/ui` to buy
structure this change writes in ~400 lines.

**`cursive`.** Retained-mode with its own backends. Would replace ratatui
outright. Not proportionate.

**`tui-textarea` / `tui-input`.** The strongest candidate, since text editing is
the one place with genuine hidden complexity (graphemes, wide characters,
soft-wrap). Rejected for the same reason `todo-editing-keyboard` rejects it: it
carries its own keymap and rendering, which we would then be overriding key by
key. Worth revisiting if grapheme-correct editing becomes a requirement — the
`TextField` boundary is drawn so it could be swapped behind it.

**Do nothing and keep copy-adapting.** Defensible while the overlay count is
small; the count is now nine and two of them are byte-for-byte parallel. The
cost of this change is paid once; the cost of not making it is paid per overlay,
and every payment is a chance for the render and the hit-test to disagree.

**Do the descriptor (group 5) first.** It is the highest-value piece — it is what
makes the help-panel and input-source rules structural. It is also the piece most
likely to conflict on an upstream sync, and doing it first means doing it without
the snapshot harness that would prove nothing moved. Last, behind the harness.
