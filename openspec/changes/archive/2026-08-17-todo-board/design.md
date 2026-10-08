# Design

## Alongside the panel, not instead of it

The board could replace the per-pane panel — one todo surface, filtered to a pane
when you want that. Rejected: the panel is anchored to its pane and opened from
that pane's indicator, which is what makes "add a todo about *this*" a single
gesture. A filtered board loses the anchor and turns a one-key action into a
navigation. The two surfaces answer different questions ("what about this pane?"
versus "what is outstanding?") and both are cheap because they share the store,
the ordering and the row rendering.

The cost accepted: two overlays that look similar and are opened by different
keys. Mitigated by making the row identical in both and the keys identical where
the action is identical, so the board reads as the panel widened rather than a
second feature.

## Grouping and ordering

Grouped by owning pane, in space > tab > pane order — the order the sidebar and
the navigator already present the session in, so a third ordering is not
invented. Within a pane, the existing presentation order (not-done before done,
then priority descending, then creation order) applies unchanged.

Ordering *across* panes is deliberately not by priority. A global priority sort
reads better in a screenshot and worse in use: it scatters a pane's todos
through the list, so the common act of dealing with one pane's work means
hunting. Grouping keeps a pane's todos contiguous and lets the eye skip whole
panes. This is a real choice and belongs in the spec, not just here.

Panes with no todos SHALL NOT appear. A board that lists every pane in the
session to say "nothing here" buries the todos it exists to show.

## What a row shows

The panel's row plus its owner. The panel can leave the pane implicit — you
opened it from that pane — while the board cannot. The owner goes in the group
heading rather than on every row, so the row itself stays the panel's row and the
two surfaces do not drift.

Group headings carry the pane's identity the way the link picker learned to
(`pane-todos-ux`): the addressable id leading, then the label, so a heading names
a pane the user can actually find. Headings are not selectable; selection steps
over them, as in the move picker.

## Activating a row focuses its pane

The board's whole justification over `herdr todo list --all`. Reuses the focus
path the existing link-following and notification-jump already use, so there is
one definition of "go to that pane" rather than a third.

This creates an ambiguity the panel does not have: a todo with a link has *two*
destinations — its owning pane and its linked pane. Resolved by keeping the
panel's `g` for the link and giving the board's primary activation (Enter) the
owner. Enter is the row's own meaning; `g` stays the link's, exactly as it reads
in the panel.

## Closing on activation

Focusing a pane closes the board. A board left open over the pane you just jumped
to is in the way, and the state it holds (selection) is cheap to rebuild. This
matches the navigator, which closes on submit.

## Empty state

The board opens even with no todos anywhere, showing an empty state rather than
refusing. Refusing to open is indistinguishable from a broken keybinding — the
same reasoning that made the pane panel keep its footer when a pane is empty
(`pane-todos-ux`).

## Geometry

A centred modal rather than an anchored panel: it belongs to the session, not to
a pane, so anchoring it to one would misrepresent what it shows. Built from the
existing modal shell, header and footer-button row, and reserving the footer
block (`FOOTER_ROWS`) so the buttons keep the blank row above them.

`overlay-ui-kit` landed first (archived 2026-08-16), so the folding this section
anticipated is not deferred work — the kit is what the board is built on from the
start. Concretely the board takes its footer row from `ButtonRow` / `ButtonSpec`,
its selection and scroll from `ListCursor`, its movement chords from
`list_chord`, and its help entries from the `overlay_help` match, and it is a
variant in the `overlays!` list rather than another `Option<XState>` field.

The kit's `AnchoredPanelSpec` resolver is deliberately *not* used: it places a
panel against an anchor rect, and the board is centred precisely because it has
no pane to anchor to. Centred geometry stays on the existing modal path
(`centered_popup_rect` / `render_modal_shell` / `footer_split`), which is what
the pane-move picker uses and is not a bespoke triple.

## What the plan left open

Three things the tasks and the spec do not both answer, decided here before any
code moved.

**Edit cannot keep the panel's key.** Task 3.1 asks every panel action to keep
its key and task 3.2 gives `Enter` to focusing the owning pane — and on the
panel `Enter` *is* edit. The spec settles it: activating a row focusing the
owner is a requirement with a scenario, "same key where the action exists" is a
principle. So edit is `e` on the board, and `Enter` is the row's own meaning.
Nothing else moves: `space`, `g`, `d`, `c`, `esc`/`q` are the panel's.

**`clear done` clears one pane, not the session.** At session scope the label
could mean either. It clears the done todos of the *selected todo's owning
pane*, because that is what the key already means one surface over — a
session-wide sweep would be a different and far more destructive action wearing
the same letter. The selection is always visibly inside a pane's group, so what
`c` will clear is on screen.

**A row click activates.** On the panel a row click opens the edit view,
because that is the row's own meaning there; on the board the row's own meaning
is its owner, so a click focuses the owning pane and a click on the link chip
follows the link — the same rule (the row does what the row means), not the
same outcome.

## Alternatives considered

**A sidebar section instead of an overlay.** The sidebar is already the session's
standing surface and todos would be visible without a keystroke. Rejected for
now: the sidebar's vertical budget is contested (spaces, agents), and todos are
consulted in bursts rather than watched continuously. An overlay costs one key
and no permanent space.

**Reusing the notification center's shape.** Superficially similar — a list of
things with a footer — but the notification log is flat and time-ordered while
todos are grouped and priority-ordered, and its rows carry read state rather than
done state. Sharing the widgets is right; sharing the surface is not.
