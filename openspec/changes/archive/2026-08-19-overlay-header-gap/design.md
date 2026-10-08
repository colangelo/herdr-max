# Design

## The header block mirrors the footer block

`FOOTER_ROWS = 2` is the button row plus the blank row above it, and it lives in
the kit rather than at each call site because the panels that sized their
content as "everything but the footer" each lost the gap that way.
`HEADER_ROWS = 2` is the same shape at the other end: the title row plus the
blank row under it.

`header_split(inner)` serves the overlays that place rows by offset from
`inner` — the board, the pane-move picker, the open-worktree dialog. The ones
built from an explicit `Layout::vertical` spend the same rows as a
`Constraint::Length(1)` spacer instead, because threading a helper through a
constraint list would obscure rather than share.

The pane-move picker's header is two lines — a title and a subtitle — so it
gets `PANE_MOVE_TARGET_HEADER_ROWS = HEADER_ROWS + 1`, named rather than
open-coded because its renderer and its mouse hit-test both consume it and must
not drift.

## Why the heading carries no identifier and the link chip does

The heading is `infra · imap-jmap-mcp`, while the todo link chip stays
`→ w2:pC · claude`. That looks inconsistent and is deliberate.

The heading carried the identifier first, on the reasoning that a heading owns a
whole row and can afford the address. Dogfooding the board found the address was
not worth a row at any price. `w5:pV` is space 5, pane `V`, where `V` is 27 in
the 32-character alphabet `123456789ABCDEFGHJKMNPQRSTVWXYZ0` — I, L, O and U are
skipped so nothing is misread as 1 or 0. That 27 is a creation counter, never
reused, and not a position: you cannot tell which pane it is by reading it, and
`p27` would be readable and exactly as meaningless. The heading also named the
space twice, because `w5` and the space's own name are the same fact.

The encoding itself is untouched. It is upstream's (`f7a7da03`, upstream #569),
it is what `$HERDR_PANE_ID` exposes, and re-encoding it here would diverge the
fork and conflict on every sync for a cosmetic gain.

A heading's job is recognising the pane, which the space and the label do, and
activating the row travels there without anyone reading an address. A chip is
the other case: it is a destination you may want to address — with `herdr pane`,
`herdr agent read`, or a sibling agent's prompt — and it shares its row with the
todo's own text and is truncated from the right, so the identifier goes first
there to survive.

The rule this leaves behind: show an identifier where it will be used, not where
a row happens to have room for it.

## Why the space, and why first

The board is read across spaces — that is its entire justification over the
per-pane panel. The heading previously named the space only as the first
character of `w2:pP`, which is an identifier to decode rather than a name to
recognise. The space name is what the sidebar already calls it, so the board
borrows that rather than inventing a second name for the same thing.

## Reversing the board's `clear done` scope

`todo-board` decided that `c` on the board should clear the *selected* todo's
pane, so the key would mean exactly what it means on that pane's own panel, and
so a session-wide sweep would not hide behind a familiar letter. Dogfooding
overturned it within a day.

The failure mode is that a scoped action with no visible scope is
indistinguishable from a broken one. The footer says `clear done`. Press it with
the selection parked on a pane whose todos are all outstanding — the common case,
since the selection opens on the first todo in the list — and nothing happens and
nothing is said. It was reported as "c does not work", which is the correct
reading of what it did.

So `c` now clears the completed todos of every pane the board is showing. The
destructiveness argument does not survive contact either: the todos being removed
are the ones already marked done, the board is showing exactly which, and the
per-pane panel still offers the narrower version for anyone who wants it.

The general rule this leaves behind: an action on a session-scoped surface takes
the scope of that surface, or it must show its scope. It cannot quietly take a
narrower one.

## Alternatives considered

**A blank row under every title, including the ones that already have one.**
Rejected as a non-change: the four conforming overlays already look right, and
touching them would re-record snapshots for no visual difference.

**Keeping the identifier, last, after the space and the label.**
`infra · imap-jmap-mcp [w2:pP]` is what shipped first and what dogfooding
removed. Nothing reads wrong about it, which is why it survived review — the
row had space, so the field went in. Space in a row is not a reason to fill it.

**Rendering the counter in decimal — `infra · imap-jmap-mcp [p27]`.**
Legible, and no more useful: 27 is still the order the pane was created in
rather than anything on screen. It would also have to be decoded back for
`$HERDR_PANE_ID`, putting a fork-local spelling on an upstream identity.
