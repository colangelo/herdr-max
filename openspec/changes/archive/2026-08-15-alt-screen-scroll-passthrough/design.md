# Design — alt-screen scroll passthrough

## Decision 1: trigger on `alternate_screen_active()`, not the page-key predicate

The plain-`PgUp` interception uses
`InputState::plain_page_keys_use_host_scrollback()`, which is deliberately
broader than the alternate-screen flag (a primary-screen pager like `less -X`
keeps the primary screen but owns the page keys). The diversion here uses the
**narrower** condition — the terminal's alternate-screen flag, read through the
existing narrow accessor `TerminalRuntime::alternate_screen_active()`:

- Alt-screen active ⇒ scrollback provably frozen ⇒ copy-mode scroll entry is a
  dead end ⇒ divert.
- Primary-screen pager ⇒ the pane's scrollback is real and copy mode over it is
  useful ⇒ keep copy mode, exactly as today.

If no runtime can be resolved for the focused pane, fall through to the
existing copy-mode entry (which already declines safely).

This is a per-gesture check on one pane — not render-loop or parse-loop work —
so the multiplicative-performance rules are not in play. The narrow accessor is
still preferred over collecting a full `InputState`.

## Decision 2: a real mode, state-owned, with a pending-send effect queue

`AppState` is pure data; sending bytes to a PTY is a runtime effect. The house
pattern for this is already in the tree: copy mode queues clipboard writes that
`App::dispatch_pending_clipboard_write()` drains. The passthrough mode does the
same:

- `Mode::AppScroll` joins the `Mode` enum. `AppState.app_scroll:
  Option<AppScrollState { pane_id }>` pins the pane, and the
  mode/state pairing follows the same invariants style as `copy_mode`
  (`app_scroll` is `Some` iff mode is `AppScroll`; covered by
  `assert_invariants_for_test` if a matching invariant exists for copy mode).
- Deciding to divert, entering the mode, and translating keys are `AppState`
  methods; each key to forward is pushed to a pending queue
  (`AppState.pending_app_scroll_keys: Vec<TerminalKey>`).
- The `App` layer drains the queue after dispatch: resolve the pinned pane's
  runtime, `encode_terminal_key(key)`, send through `lookup_runtime_sender`.
  Both live dispatch paths (`App::execute_tui_navigate_action` and the
  state-level `execute_navigate_action_in_context`) therefore behave
  identically — the state-level path queues, and the draining caller sends.

Encoding through `encode_terminal_key` is what the ordinary forwarding path
does (`prepare_terminal_key_forward`), so applications negotiating the kitty
keyboard protocol receive the same bytes a physical key press would produce.

The drain deliberately does **not** route through
`prepare_terminal_key_forward`: that path re-applies direct-binding
interception and the plain-page-key transcript heuristic. The mode's contract
is "this key goes to the application", so it encodes and sends, nothing else.

## Decision 3: key vocabulary is the pager vocabulary, nothing more

| In the mode           | Sent to the application |
| --------------------- | ----------------------- |
| `ctrl+u`, `PgUp`      | `PageUp`                |
| `ctrl+d`, `PgDn`      | `PageDown`              |
| `ctrl+k`, `k`, `Up`   | one wheel-up tick       |
| `ctrl+j`, `j`, `Down` | one wheel-down tick     |
| `g`, `Home`           | `Home`                  |
| `shift+g`, `End`      | `End`                   |
| `Esc`, `q`, `Enter`   | nothing — exit the mode |
| prefix chord          | nothing — enter prefix mode (as copy mode does) |
| anything else         | nothing — swallowed (as copy mode does) |

Entry gestures map the same way: the page and half-page gestures send one
`PageUp` on entry ("half" vs "full" has no distinct terminal key; Claude Code
pages by half-screens on `PgUp` anyway), the line gesture sends one wheel-up
tick.

Line granularity cannot ride on a key: no universal line-scroll key exists,
and forwarding arrow key presses would type into prompt history in shell-like
TUIs. Instead a wheel tick is synthesized and encoded exactly as herdr already
encodes a physical wheel event over that pane (`wheel_routing()`): a mouse
report at the pane's centre for mouse-capturing applications (Claude Code
scrolls a few lines per tick), `encode_alternate_scroll` arrows under
DECSET 1007 (which ghostty-vt defaults on for alt-screen apps), and dropped
when neither path exists. The arrow keys are safe to bind here because they
map to wheel ticks, never to forwarded arrow presses.

Key-release events are ignored on entry to the handler, mirroring
`handle_copy_mode_key`; synthesized keys are press events.

## Decision 4: exit is cheap and defensive

`Esc`/`q`/`Enter` exit to `Mode::Terminal`. On every key, if the pinned pane is
no longer the focused pane (closed by another client, focus stolen via the
API), the mode exits instead of forwarding — the same defensive posture as
`sync_copy_mode_with_focus`. Nothing needs restoring on exit: unlike copy mode
there is no viewport anchor, because the viewport never moved — the
application did its own scrolling.

## Decision 5: the indicator rides the focused pane

While the mode is active the focused pane visibly says so and names the exit
key (wording like `scroll · esc`), reusing the pane-border/overlay styling
already used for pane badges rather than inventing a new surface. Without it, a
swallowed keystroke reads as a hang.

## Alternatives considered

- **Rebinding inside Claude Code** (`~/.claude/keybindings.json`, `Scroll`
  context, `scroll:halfPageUp`). Works for one application the user configures,
  does nothing for the next alt-screen TUI, and risks shadowing `ctrl+u`
  kill-line in the chat input. Kept as a user-side option, not the fix.
- **One-shot forwarding without a mode** (each `prefix+ctrl+u` sends one
  `PageUp`). Loses the repeat ergonomics the copy-mode gestures established —
  reading back a long transcript would cost one prefix chord per half page.
- **Forwarding from inside copy mode when scrollback runs out.** Blends two
  input contracts (keys-to-herdr vs keys-to-app) inside one mode; a selection
  cursor and an app that scrolls underneath it cannot coexist coherently.
- **Translating via `encode_alternate_scroll` / DECSET 1007 arrows.** That
  encoding exists for wheel events and only when the application opted into
  alternate-scroll mode; Claude Code captures the mouse instead. Page keys are
  the universal vocabulary.
