# Alt-screen scroll passthrough for the copy-mode scroll gestures

## Why

The copy-mode scroll entry gestures (`copy_mode_page_up`,
`copy_mode_half_page_up` = `prefix+ctrl+u`, `copy_mode_line_up`) promise "scroll
back from the home row". On a pane whose application owns the **alternate
screen** they cannot keep that promise: while the alternate screen is active the
pane accumulates no scrollback, so the gesture enters copy mode on a buffer
whose top edge is the visible screen. The cursor climbs a few rows and stops.
The gesture is not merely degraded — it is a dead end, every time, on every
alt-screen pane.

This got acute because Claude Code's fullscreen renderer (an alternate-screen
TUI, like `vim` or `htop`) is now the saved default for many users. Those
applications scroll *themselves* — Claude Code pages with `PgUp`/`PgDn` — but
the dedicated page keys are exactly what home-row gestures exist to avoid.
Herdr already understands this split for other inputs: plain `PgUp` is
intercepted for pane scrollback only when
`InputState::plain_page_keys_use_host_scrollback()` says the pane looks like a
shell transcript, and mouse wheel events on alt-screen panes are translated to
arrow keys (`WheelRouting::AlternateScroll`). The keyboard scroll gestures are
the one scroll surface that still ignores pane state.

## What Changes

- When a copy-mode scroll gesture fires on a pane whose terminal is in the
  **alternate screen**, Herdr enters a lightweight **app-scroll passthrough
  mode** on that pane instead of copy mode. Entry forwards the scroll intent to
  the application (`PageUp` for the page and half-page gestures; the line
  gesture enters without sending).
- While the mode is active, scroll keys keep working from the home row and are
  forwarded to the application: `ctrl+u`/`PgUp` send `PageUp`, `ctrl+d`/`PgDn`
  send `PageDown`, `g`/`Home` send `Home`, `shift+g`/`End` send `End`. `Esc`,
  `q`, or `Enter` exit back to terminal input; the prefix chord still opens
  prefix mode. Other keys are swallowed, mirroring copy mode.
- Forwarded keys are encoded through the pane's own terminal state
  (`encode_terminal_key`), so applications using the kitty keyboard protocol
  receive well-formed events.
- The focused pane shows a visible indication while the mode is active, naming
  the exit key, so swallowed keys are never a mystery.
- Panes on the primary screen are untouched: the gestures enter copy mode
  exactly as before, including the no-scrollback edge case.
- `copy_mode` itself (plain entry, default `prefix+[`) is untouched everywhere:
  selecting visible text on an alt-screen pane stays possible.

## Impact

- Affected specs: `copy-mode-scroll-entry` — `One-gesture copy-mode entry with
  upward scroll` modified (alt-screen diversion), `Alternate-screen scroll
  passthrough mode` added.
- Affected code: `src/app/state.rs` (new `Mode` variant + passthrough state +
  pending key-send effect, mirroring the pending-clipboard pattern),
  `src/app/input/copy_mode.rs` (entry diversion), `src/app/input/navigate.rs`
  (both dispatchers), `src/app/input/mod.rs` (mode routing),
  `src/app/input/terminal.rs` or `src/app/creation.rs` (drain: encode + send),
  `src/ui/panes.rs` (indicator), docs under
  `docs/next/website/src/content/docs/`.
- **No wire changes.** The mode is TUI client state; keys reach the pane
  through the existing input send path. No API field, protocol message, or
  config key is added, and no keybinding moves, so the `prefix+?` help panel
  keeps its existing entries.
- Behavior change for existing users, deliberate and narrow: on alt-screen
  panes only, the three scroll gestures now scroll the application instead of
  entering copy mode against an empty scrollback. Anyone who wants copy mode on
  such a pane still has plain `copy_mode`.
- Fork tracking: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/59.
  Designed to be upstream-PR-able: no fork-specific naming, no
  application-specific detection — the trigger is the terminal's own
  alternate-screen flag.

## Non-goals

- Line-granular passthrough (`j`/`k` forwarding arrow keys). Arrow keys mean
  prompt history in Claude Code and shell-like TUIs; forwarding them from a
  scroll mode would type into the application. The line gesture enters the mode
  and the paging vocabulary takes over.
- Mouse changes. Wheel routing already handles alt-screen panes.
- A config knob to disable the diversion. The diverted gesture had no useful
  behavior on alt-screen panes; there is nothing to preserve. A knob can be
  added later if a real workflow surfaces.
- Emulating scrollback for alt-screen applications, or anything
  application-specific (no Claude-Code detection, no manifest coupling).
