# Group 6 roundtable — one overlay value

`AGENTS.md` classes group 6 as refactor-risk: it touches core state, UI/input
state projection, and identity. This is the roundtable it asks for, held before
any code moved, plus task 6.1's inventory of protected behaviour.

## 6.1 Protected behaviour, and what already holds it

| Behaviour | Where it lives now | Characterization |
|---|---|---|
| Mode/state pairing — an open overlay's mode and its state agree | `Mode` plus ten-plus parallel `Option<XState>` fields on `AppState`, paired by convention; every accessor defends with `?` | **Missing.** Added in 6.7 as an `AppState::assert_invariants_for_test` clause driven from `AppState::test_with_adversarial_identity_state` |
| Input-source selection | `Mode::wants_ascii_input` (`src/app/state.rs`), a hand-maintained allowlist of 13 modes | `sync_prefix_input_source` tests; extended in 6.4 with a per-variant assertion |
| Key-repeat gating | `App::terminal_input_context` (`src/app/mod.rs`), an if-chain over `Mode` | `src/app/input/lease.rs` tests |
| Help-panel coverage | `keybind_help_groups` (`src/ui/keybind_help.rs`), a hand-written list, with `AGENTS.md` asking humans to remember it | **Missing.** Added in 6.5 as a guard test over every overlay variant |
| Mouse dispatch on mode | `AppState::handle_mouse` branches on `self.mode` | The per-overlay mouse tests in `src/app/input/mouse.rs`, plus group 1's rendered-layout snapshots |
| Overlay layout | The kit's geometry, rows and cursors | Group 1's snapshots, unchanged through groups 2–5 |

## What the roundtable found

### `pane_todo_edit` deliberately outlives its mode, and the plan did not account for it

`open_pane_todo_link_picker_from` opens the navigator *over* an open todo edit:
its doc comment says "`pane_todo_edit` lives outside `mode`, so it survives
behind the picker and dismissing simply returns to it with the staged link
untouched." So `Mode::Navigator` with `pane_todo_edit == Some` is not a bug to
be made unrepresentable — it is a shipped feature.

A flat `Option<Overlay>` would delete it. The fix is to model the suspension
rather than forbid it: while the navigator is open for
`NavigatorPurpose::PaneTodoLink`, the navigator variant *carries* the suspended
edit. Closing the picker hands it back. That satisfies the spec's "two overlays'
states SHALL NOT be present at once" strictly — there is one overlay, and what
it is doing is part of its own state — and it makes the return path explicit
instead of implicit in a field nothing clears.

### Not every mode is an overlay, and `Mode` stays

`Terminal`, `Prefix`, `Copy`, `Navigate`, `Resize` and `AppScroll` are modes:
they have no modal surface, they are what the terminal is doing. `copy_mode`
(172 references) and `app_scroll` stay where they are, and `Mode` keeps its
`wants_ascii_input` arm for the non-overlay modes it still answers for.

### Task 6.4 is factually wrong about the second allowlist

There is no `Mode::honors_key_repeat`. Key repeat is gated by
`App::terminal_input_context`, an if-chain that returns `Some` only for
`Terminal`, `Copy`, `AppScroll` and a popup pane, and `None` for everything
else. It is therefore *already* structural: no overlay honours key repeat, and
there is no allowlist an overlay can silently fall off. It lives on `App`
rather than `AppState` because the popup arm needs `popup_pane`. 6.4 delivers
the `wants_ascii_input` half and records this.

### Help entries derive from the variant without moving the panel's grouping

The help panel is grouped by topic — global, navigation, panes, workspaces /
tabs — which is how a user reads it, not how the state is modelled. Deriving
entries per overlay must not turn the panel into a list of overlays. Each
variant therefore contributes `(group, entry)` pairs that
`keybind_help_groups` merges into the existing topics, and the guard test
asserts every variant contributes at least one. The panel renders identically;
group 1's snapshot proves it.

### The state that is not `Option` today

`navigator`, `keybind_help`, `settings` and `global_menu` are always-present
fields, but every one of them is fully reset by its open function, so nothing
is lost by making them live only while their overlay is open. Two call sites
read them outside their mode and become `Option`-aware:
`handle_mouse_event` samples `settings.section` either side of dispatch to
notice a move into Integrations, and `cancel_settings` takes
`original_palette` / `original_theme` — which must therefore run *before* the
overlay is dropped, not after.

### Fork surface

Groups 1–5 reduced fork lines. This one adds conflict-prone edits to `Mode` and
`AppState`, which are what upstream edits most. That was known when the change
was written and is why this group is last. It is recorded for
`herdr-sync-upstream` in task 7.3.

## Decisions

1. `Overlay` carries every mutually exclusive overlay's state; `Mode` stays and
   the variant supplies it.
2. The navigator variant carries a suspended todo edit rather than
   `pane_todo_edit` outliving its mode.
3. `copy_mode`, `app_scroll` and the non-overlay modes are out of scope.
4. `wants_ascii_input` derives from the variant; `terminal_input_context` is
   documented as already structural and left alone.
5. Help entries derive from the variant into the panel's existing topic groups.
6. Accessors keep their names, so the ~350 call sites move mechanically and the
   diff stays reviewable.
