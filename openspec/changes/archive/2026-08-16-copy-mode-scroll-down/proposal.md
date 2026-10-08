# Scroll back down in one gesture

## Why

Every scroll-entry gesture Herdr has goes up: `copy_mode_page_up`
(`prefix+pageup`), `copy_mode_half_page_up` (`prefix+ctrl+u`), and
`copy_mode_line_up` (`prefix+ctrl+k`). There is no downward counterpart, and
`prefix+ctrl+d` / `prefix+ctrl+j` are unbound.

That is survivable in copy mode, where the mode lingers behind the prefix and
`ctrl+d` works bare. It is a dead end in the alternate-screen passthrough mode,
which exits on the prefix chord by design: switching spaces goes through the
prefix, so coming back the mode is gone while the application is still scrolled
up. The only way back in is a gesture that scrolls further **up**, so reaching
the bottom means first going the wrong way. Reported from use:

> I scroll up with prefix+ctrl+u, switch to another space, come back, press
> prefix+ctrl+d expecting to go down — nothing. I have to go up first, and then
> I can go down.

Leaving the mode with `esc` or `q` while the application is scrolled up reaches
the same dead end.

## What Changes

- **Three downward actions mirroring the upward ones**: `copy_mode_page_down`
  (default `prefix+pagedown`), `copy_mode_half_page_down` (default
  `prefix+ctrl+d`), and `copy_mode_line_down` (default `prefix+ctrl+j`) — the
  keys users already reach for, all currently unbound. Rebindable, unbindable,
  in the `prefix+?` help panel and the config template like every other entry.
- **On an alternate-screen pane they enter the passthrough mode going down**,
  forwarding `PageDown` or a wheel-down tick, so the mode can be re-entered
  from either direction.
- **In copy mode they scroll the viewport down**, the exact mirror of the
  upward gestures, preserving the entry scroll anchor the same way.
- **On a primary-screen pane with no copy mode active they do nothing.** There
  is nothing below a live viewport, so entering copy mode to scroll down zero
  lines would strand the user in a mode they did not ask for.

## Impact

- Affected specs: `copy-mode-scroll-entry` (downward requirement added).
- Affected code: `src/app/input/copy_mode.rs` (entry gains a direction),
  `src/app/input/app_scroll.rs` (passthrough entry gains a direction),
  `src/config/` (three bindings), `src/app/input/navigate.rs` (three actions),
  `src/ui/keybind_help.rs`, `src/main.rs` template, and docs.
- No API, protocol, or wire changes.
- Fork tracking: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/64.
  Designed to be upstream-PR-able: mirrors an existing feature, neutral naming.

## Non-goals

- Keeping the passthrough mode alive across the prefix chord. Exiting is
  deliberate ("no anchor to preserve"), and with a downward entry the round trip
  costs one gesture rather than being impossible.
- Tracking an alternate-screen application's scroll position. Herdr forwards
  scroll intents blindly and cannot know where the application is.
- Changing the upward gestures, plain `copy_mode` entry, or the in-mode key
  vocabulary, which already handles both directions.
