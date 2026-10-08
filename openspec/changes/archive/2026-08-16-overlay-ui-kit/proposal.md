# A shared overlay kit for panels, modals, and pickers

## Why

Herdr's overlays — modals, panels, pickers, menus, the notification center, the
todo panel — are built on ratatui primitives (`Block`, `Paragraph`, `Rect`) plus
`src/ui/widgets.rs`. Ratatui deliberately ships no overlay, focus, or component
model, so every ratatui app grows its own layer there. Herdr's has grown by
copy-adapt, and the duplication is now measurable rather than theoretical.

**Panel geometry is written once per panel.** `notification_center_rect`
(`src/app/input/mouse.rs:1471`) and `pane_todo_panel_rect` (`mouse.rs:1589`) are
each ~60 lines performing the identical sequence: pick an anchor, measure the
widest content row, `.clamp(30, 60)`, count rows, reserve a footer, place x and
y against the screen, derive the inner rect, derive the button rects, derive the
list window, derive `row_at`, derive `footer_row_y`, derive `covers`. They differ
in what they measure and where they anchor, and in nothing else. A third panel
means a third copy.

**Button rows are written once per panel.** `PaneTodoPanelButton` +
`pane_todo_panel_button_rects` + `hit` + `row_y` (`src/ui/todo_panel.rs:45-121`)
is the same code as `NotificationCenterButton` +
`notification_center_button_rects` (`src/ui/notification_center.rs:41-100`),
differing only in the enum it returns.

**List cursors are written per overlay, and there are already two.**
`MenuListState` and `SelectionListState` (`src/app/state.rs:1132`, `:1159`) are
the same struct with a different field name; several overlays use neither and
keep a bare `selected: usize`. Scroll windowing is re-derived per overlay.

**List keys differ per overlay by accident.** The navigator's list state moves
on `j`/`k` and, because those arms carry no modifier guard, on `ctrl+j`/`ctrl+k`
too; its search state moves only on the arrows and `ctrl+n`/`ctrl+p`. Nothing
records which set an overlay is supposed to offer, so each new one re-decides.

**Mode and state are paired by convention only.** `Mode` has 24 variants
(`state.rs:815`) and `AppState` holds ten-plus parallel `Option<XState>` fields.
Nothing prevents `mode == PaneTodos` with `pane_todos == None`, or two overlay
states being `Some` at once; every accessor defends with `?`. Three cross-cutting
behaviours — `Mode::wants_ascii_input`, `Mode::honors_key_repeat`, and "is this
action in the keybind help panel" — are hand-maintained allowlists a new overlay
can silently miss. Two of them say so in their own doc comments; the third is
enforced by asking humans to remember it in `AGENTS.md`.

One thing in here is already right and is the reason this is a tidy-up rather
than a rewrite: geometry lives on `AppState` and *both* the renderer and the
mouse hit-test call it, so what is drawn and what is clickable cannot drift.
`notification_center_list_window` says so in its own comment. That property is
preserved and made structural.

## What Changes

- **One anchored-panel geometry.** A spec (anchor, measured content width, row
  count, row cap, footer rows, corner preference) resolves to a geometry (outer,
  inner, list, footer row). Both existing panels are expressed in it and their
  bespoke rect functions are deleted.
- **One footer button row**, generic over the overlay's button enum, producing
  the rects, the hit-test, and the row's y from one definition.
- **One list cursor**, with windowing, replacing `MenuListState` and
  `SelectionListState`, and adopted by the overlays that currently keep a bare
  index.
- **One list keymap**, so every list-bearing overlay accepts the same chords in
  the same states, including while a search box is focused.
- **One text field**, the primitive `todo-editing-keyboard` introduces for todo
  composition, adopted by the rename modals, worktree create, and the navigator
  and keybind-help search boxes — which today each reimplement their own
  append-only editing.
- **One overlay descriptor.** The parallel `Option<XState>` fields collapse into
  a single `Option<Overlay>` enum whose variants carry today's state structs, and
  the three cross-cutting allowlists derive from it instead of being restated.
- **A snapshot harness for overlays**, promoting the `row_text(buffer, rect)`
  helper already in `todo_panel.rs`'s tests into a shared utility, so each
  overlay gets a cheap rendered-layout regression test.

## Impact

- Affected specs: `tui-overlay-kit` (new capability).
- Affected code: `src/ui/widgets.rs` (grows the kit or is split into
  `src/ui/overlay/`), `src/ui/{todo_panel,notification_center,menus,navigator,
  settings,keybind_help,release_notes,onboarding}.rs`, `src/app/input/mouse.rs`
  (the two rect functions and their derivations), `src/app/state.rs` (`Mode`, the
  overlay state fields, the two list cursors), `src/app/input/{mod,modal,
  overlays,settings}.rs`.
- **No user-visible behaviour change is intended** beyond list chords becoming
  uniform, which is a strict superset of what each overlay accepts today. No
  wire changes: no schema field, protocol version, snapshot shape, config key, or
  CLI flag moves.
- **No new dependencies.** `tui-textarea`, `tui-realm`, and `cursive` were all
  considered and rejected in the design; the kit is ~400 lines of in-repo code.
- This is fork-only code, so it is rebase surface on every upstream sync. Groups
  1–4 *reduce* total fork lines, which lowers that surface. Group 5 raises it: it
  touches `Mode` and `AppState`, which upstream also edits.
- Group 5 is **refactor-risk** under `AGENTS.md`: it touches core state, UI/input
  state projection, and identity. It requires characterization tests named before
  the move, `AppState::assert_invariants_for_test` with
  `AppState::test_with_adversarial_identity_state`, and a roundtable. Groups 1–4
  are ordinary local refactors.

## Non-goals

- `src/ui/sidebar.rs` (3,715 lines) and `src/ui/panes.rs` (2,013 lines). They are
  a different problem and this kit will not shrink them.
- Adopting a component framework, or changing the immediate-mode model. `render`
  stays pure and takes `&AppState`; the kit is data and geometry, not a widget
  tree with its own state.
- Any visual redesign. Every overlay should render identically, cell for cell,
  before and after — which is what the snapshot harness is for.
- Rewriting overlay *keymaps* beyond the list chords. Which key does what inside
  a given overlay stays that overlay's business.
