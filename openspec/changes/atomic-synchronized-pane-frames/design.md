## Context

The fork's server renders each attached client's whole screen with ratatui
(`render_virtual_with_runtime_registry`), diffs it against the client's last
frame and sends the difference. A retained path (`try_retained_*` in
`src/server/headless.rs`) sends only a pane's dirty rows when nothing else
changed. Both read each pane's live grid (libghostty-vt) at build time. The
vendored libghostty-vt tracks mode 2026 but has no timeout of its own (Ghostty
keeps that in its termio layer, 1 s).

## Decisions

### Hold the whole client frame, not one pane (recommended)

Two ways to keep a mid-block pane from showing:

1. **Hold the client's whole frame** while a pane it shows is mid-block
   (upstream's choice). Cheap: one state read per visible pane per frame, no
   copies. Cost to the user: while an app is inside a block, the rest of that
   client's screen waits too. Real blocks last a few ms (Claude Code writes a
   frame in well under 20 ms), and the end of the block draws at once.
2. **Hold only that pane**, drawing its last complete cells from a cache while
   the rest of the frame goes out live. Nicer when one app sits in long
   blocks, but it needs a copy of every sync-using pane's cells on each
   complete render (W x H cells per pane per frame, times panes, times
   clients) - a multiplicative path cost - plus rules for a pane whose size
   changed under the cache.

Choice: **1**, with the timeout below as the cap on how long anything waits.
If ac later sees a real app hold the screen, 2 can be added behind it.

### Timeout: 200 ms

A block that is still open after `SYNC_HOLD_MAX = 200 ms` stops holding
frames; herdr draws the pane as it is (today's behaviour, never worse). 200 ms
is ten times a large real frame and short enough that a stuck app never
freezes the screen noticeably. Ghostty's 1 s is for a single-app window; here
one app would hold every pane on the client. The begin of a block returns a
render delay of 200 ms, so the timeout fires with no input.

### Epoch against a block that starts during the build

The check and the build read each pane under separate locks while PTY threads
keep writing. Each pane's core counts begins and ends (`epoch`, wrapping u64).
The frame records the epochs it checked; after the build, if any shown pane's
epoch changed or it is now mid-block, the frame is dropped and a render is
requested. Same technique as upstream.

### Where the check lives

A pure helper over `&AppState` + `&TerminalRuntimeRegistry` returns `Hold {
until }` or `Draw` for a target (tab + popup). It reads only the panes of that
target (`pane_infos` / the tab layout), so hidden tabs cost nothing, and it
takes the one core lock per pane that `synchronized_output_active` already
takes. No allocation.

## Risks

- A client could see a slightly later frame for input typed in another pane
  while an app is mid-block: bounded by the block length, max 200 ms.
- Resize while mid-block: the resize still applies (not held; upstream skips
  it, the fork keeps it simple). The next complete frame draws the new size.

## Testing

- Terminal: the epoch changes on begin and on end, not on plain output; the
  begin asks for a render after `SYNC_HOLD_MAX`.
- Helper: a target with a mid-block pane holds; after the deadline it draws; a
  hidden tab's mid-block pane does not hold the visible tab; the popup counts.
- Server: a full render for a client whose tab has a mid-block pane sends no
  frame and owes one; the block's end sends it; a retained patch for a
  mid-block pane falls back.
- Reproducer: the #126 throwaway measurement goes from 38% torn to 0 at the
  middle with keys sent, with a beta.
- `just bench-render-scale` 1 vs 15 panes, before and after.

## Implementation notes (2026-09-29)

- The epoch is one global transition count (`SYNC_TRANSITIONS` in
  `src/pane/terminal.rs`) plus, per pane, the count at its last begin or end.
  A frame reads the count before it is built; after the build a shown pane
  whose last transition is newer, or whose block is open and younger than
  `SYNC_HOLD_MAX`, voids the frame. This needs no per-pane snapshot, so it
  also works when the build changed which panes are shown.
- A transition is the mode flipping across one write. A begin and its end in
  one write are atomic under the core lock and are no transition; an end and
  a new begin in one write restart the hold clock (the write contains
  `CSI ? 2026 h` while the mode stays on).
- The server builds every client frame as before (so layout and pane resizes
  still apply) and only drops the send; the client owes a full frame
  (`defer_full_render`), which also keeps the retained patch path off until a
  complete frame is sent.
- The local loop draws straight to the host terminal and cannot drop a frame
  after drawing, so it checks before drawing only.
- The hold check is in `synchronized_output_holds_frame`
  (`src/ui/tab_surface.rs`); `just bench-render-scale` now includes it in each
  sample.
