# A header block for titled overlays, and a heading that reads in order

## Why

Two things surfaced dogfooding the todo board.

**A title drawn flush against its content reads as stuck to it.** This is the
same complaint the footer had before `FOOTER_ROWS`: buttons drawn against the
last row looked crammed, so the kit reserved a blank row and every panel got it.
The top of an overlay never got the same treatment, and the overlays drifted
apart on it by accident rather than by decision — the keybind help panel, the
rename dialog, the todo edit modal and the release-notes header all leave a
blank row under the title; the todo board, the pane-move picker and the three
worktree dialogs do not. Nothing recorded which was intended.

**The board's group heading leads with the part you act on rather than the part
you think in.** It renders `w2:pP · imap-jmap-mcp`. The board exists to be read
*across* spaces, so the first question a heading answers should be "where is
this work", and the space is not in the heading at all — it is encoded in the
first character of an identifier you have to decode. Dogfooding the reordered
heading then found the identifier was not worth carrying at either end: it is a
creation counter, so it names nothing the reader can see, and its space
component repeats the space name in code.

## What Changes

`HEADER_ROWS` joins `FOOTER_ROWS` in the overlay kit: a titled overlay reserves
its title row plus one blank row under it, and the five overlays that were flush
adopt it. The four that already left the gap are unchanged — this makes the
existing majority the rule.

The board's group heading becomes `space · pane label` —
`infra · imap-jmap-mcp` — with no addressable identifier. The todo link chip
keeps its own, because a chip is a destination you may want to address rather
than a group you are reading.

## Impact

- Affected capability: `tui-overlay-kit` (one requirement added), `pane-todos`
  (the board requirement modified for the heading format)
- Affected code: `HEADER_ROWS` / `header_split` in `src/ui/widgets.rs`; the todo
  board, the pane-move picker and the new/open/remove worktree dialogs; the
  board's heading text and the projection that feeds it a space name; the
  pane-move picker's mouse row mapping, which must shift with its rows
- Five overlays grow one row taller, and their rendered-layout tests re-record
- No server, API, protocol or config surface: presentation only, client-side
  under the runtime/client guardrail

## Non-goals

- Changing the todo link chip, which keeps leading with the identifier — see
  the design note on why the two orders are deliberately different.
- A title for the pane todo panel, which has none and needs none: it is
  anchored to the pane it belongs to.
- Reflowing any overlay that already leaves the gap.
