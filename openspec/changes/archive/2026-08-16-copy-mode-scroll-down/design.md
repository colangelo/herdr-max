# Design — downward scroll entry

## Decision 1: a direction on the existing entry, not a second entry path

`enter_copy_mode_scrolled` already does the whole job — cancel stale copy mode,
divert to the passthrough on an alternate screen, enter copy mode otherwise,
then scroll — with the direction hardcoded as `-1` at the three scroll calls.
It takes a direction instead. `try_enter_app_scroll_mode` takes the same
direction and queues `PageDown` or a wheel-down tick in place of the up ones.

One path, two directions. A parallel downward path would duplicate the stale-
copy-mode cancellation and the alternate-screen divert, and the two copies would
drift.

## Decision 2: downward does not create copy mode on a primary-screen pane

Upward entry is meaningful from a live viewport: there is scrollback above it.
Downward is not — the viewport is already at the bottom — so entering copy mode
would scroll zero lines and leave the user in a mode they did not ask for,
needing `esc` to get out of a keypress that appeared to do nothing.

So the downward gestures act only where there is somewhere to go:

- copy mode active on the focused pane → scroll it down (the mirror of up)
- focused pane on the alternate screen → enter the passthrough going down
- otherwise → nothing

The "nothing" case is a real answer rather than a gap: the pane is already
showing its newest output.

## Decision 3: named as the mirror of the upward actions

`copy_mode_page_down`, `copy_mode_half_page_down`, `copy_mode_line_down`. Users
who know the upward names will guess these, the help panel lists them adjacent,
and the config template pairs them. The name stays honest because in copy mode
they do scroll the copy-mode viewport; the alternate-screen divert is the same
divert the upward actions already make.

## Decision 4: the defaults are the keys already being pressed

`prefix+ctrl+d`, `prefix+ctrl+j`, and `prefix+pagedown` mirror `prefix+ctrl+u`,
`prefix+ctrl+k`, and `prefix+pageup`, match the in-mode vocabulary the
passthrough already forwards (`ctrl+d` pages down, `ctrl+j` scrolls a line), and
are the keys the bug report was pressing. None is bound today, and none collides
with send-prefix.

## Decision 5: the passthrough still exits on the prefix chord

Unchanged. The alternative — keeping the mode alive behind prefix like copy mode
does — was considered and rejected: copy mode lingers because it owns a viewport
anchor to restore, while the passthrough owns nothing, and a mode that survives
a space switch would silently swallow the next keys typed into the application.
With a downward entry the round trip costs one gesture, which is what the
original "the gesture re-enters it in one step" comment assumed and only got
half right.

## Alternatives considered

- **Down enters copy mode anyway on the primary screen.** Symmetric on paper,
  hostile in use: a keypress that appears to do nothing and leaves a mode behind.
- **Only fix the passthrough, leave copy mode alone.** Would make
  `prefix+ctrl+d` work on an alt-screen pane and do nothing on a normal one, for
  no reason the user could see.
- **Restore the passthrough when focus returns**, the way
  `sync_copy_mode_with_focus` restores copy mode. Does not help: the prefix chord
  destroys the mode before the space switch happens, and it leaves the `esc`/`q`
  dead end untouched.
