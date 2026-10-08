# Design

## One picker, not three actions

The obvious alternative is a second keybinding — `move_pane_to_space` beside
`move_pane_to_tab` — and it is worse for the same reason the tab picker beat
three positional bindings: the user does not know, at the moment of pressing a
key, whether the destination they want is a tab here, a tab there, or somewhere
that does not exist yet. A second binding forces them to classify the
destination before they can look at the list.

So the existing `prefix+m` stays the single entry point and the *list* answers
the question. That also means no new default binding, no new chord to learn, and
nothing to add to the config surface — the change is invisible until the picker
opens.

## What an entry is

Today an entry is a tab: `{ tab_id, number, label }`. It becomes a destination,
which is one of three things:

```rust
enum PaneMoveTarget {
    Tab { tab_id: String },          // existing tab, this space or another
    NewTab { workspace_id: String }, // new tab in a named space
    NewSpace,                        // new space, created by the move
}
```

Each carries the display fields the picker already renders plus the space it
belongs to, so grouping is a property of the entry rather than a second
structure to keep in sync.

`PaneMoveTarget` maps 1:1 onto `PaneMoveDestination` at dispatch, which is the
whole point: the picker's job is to pick, and the API's vocabulary is already
correct. No translation layer, no new server concept.

## Ordering, and why the current space stays first

Grouped by space, current space first, then the remaining spaces in sidebar
order. Within a space, tabs in tab order, then that space's "new tab" entry.
"New space" sits last, alone.

The current space leads because the common move is still a short one, and a list
that reorders itself by recency would make the muscle-memory case (`prefix+m`,
first entry, enter) unreliable. The existing rule that the *source* tab is
excluded is unchanged, so the first entry is never a no-op.

Sidebar order rather than creation order because the sidebar is where the user
already reads their spaces; two different orderings for the same set is the kind
of small inconsistency that makes a picker feel untrustworthy.

## Selection is a flat list with headers, not a tree

Space headers are rendered rows but not selectable — selection steps over them.
A collapsible tree is more machinery (expand state, two-axis navigation) for a
list that is usually under twenty rows, and it would put a keystroke between the
user and a destination they can already see. If the list ever outgrows that, the
navigator is the precedent for filtering, not nesting.

## Filtering is deliberately deferred

`pane-todos-ux` replaced a cycling link control with the navigator precisely
because a session can hold dozens of panes. Spaces are far fewer, and the
grouped list is scannable at realistic sizes. Adding `/` here now would be
speculative; the entry shape above does not preclude it.

## Rejections stay where they are

The zoomed-source rejection is checked when the picker is built, and
`pane.move`'s own errors (`zoomed_tab` and friends) surface through the existing
`dispatch_pane_move_with_feedback` path. A cross-space move has no new failure
mode of its own: the server validates the destination workspace exists, and a
space that disappears while the picker is open fails the same way a tab does
today.

## New space: what it is called

`NewWorkspace { label: None, tab_label: None }` — the server's defaults, the same
name a space created any other way would get. Prompting for a name inside the
move would fuse two decisions, and the rename path (`prefix+,` on the space) is
one keystroke away afterwards. `prefix+!` already sets the precedent by creating
an unnamed tab.

## Focus follows the pane

`focus: true` on every destination, matching `prefix+!` and the current picker.
Moving a pane and being left looking at where it used to be is the behaviour
nobody wants; for a cross-space move it also means the space switch happens as
part of the move rather than as a second navigation.

## Alternatives considered

**A `--new-workspace`-style modal with a text field.** Rejected: it makes the
common case (move to an existing space) pay for the rare one, and the picker
already has to exist.

**Reusing the navigator as the picker.** Tempting — it already enumerates spaces
and tabs with good labels, and `pane-todos-ux` reused it for link picking. But
the navigator picks *panes* and its rows are pane-shaped; the destination here is
a tab or a space. Reusing it would mean teaching it a second row vocabulary and a
second submit meaning, which is how a picker becomes a mode.
