## Decisions

**Edge rows reserve space instead of overlaying a row.** Covering the first
visible row would hide a pin or the active entry, and the active entry sits at
the nearest edge exactly when the list follows focus. So a window gives up one
row at an edge only while rows are hidden past it: the top row when the window
starts past the first entry, the bottom row when entries remain past the last
one. A list that fits has neither, and `off` and `fog` keep the plain layout.
`sidebar_overflow::window` and `last_window_start` hold that arithmetic once;
both lists' visible-count, last-window, card-area and hit-test code go through
them, so render, follow-focus, scrolling and clicks agree.

**The last window is one row shorter.** Past the first window the top row is
always there, so the furthest scroll fits one row fewer. This keeps the last
entry reachable without a bottom row at the end.

**A plan, computed from layout.** `plan` takes the entries, the scroll, the
visible rects and the reserved rows, and returns the edge rows and the fog
bands. Render only paints it (fog first, as a background on the existing cells
so text keeps its colour, then the edge rows). It adds no per-pane work: one
pass over the entries' state, colour lookups only.

**Fog colour.** `lift(base, target, percent)`: the base toward the text colour,
17% then 7%. The base is what the sidebar really shows: its own background when
that is RGB; else the host terminal's background if herdr already knows it (the
OSC 11 reply kept in state, read, never queried in render); else the panel
background. With a hidden blocked or finished entry the target is the text colour
mixed 70% toward that state's colour. A colour the terminal names rather than
gives as RGB cannot be mixed, so it gets no fog.

**Urgency.** Entries are ranked by `attention_priority`; only blocked and
finished-unseen are named or tinted, since working and idle are not waiting on
the user.

## Risks

Edge rows change the window size, so the number of visible entries drops by one
or two while a list is scrolled. `off` restores the old layout.
