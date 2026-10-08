# Held keys repeat in the alt-screen scroll passthrough mode

## Why

`alt-screen-scroll-passthrough` added `Mode::AppScroll` so held-key scrolling
keeps working on panes whose application owns the alternate screen. It does
not: holding `ctrl+u`, `ctrl+d`, `ctrl+k`, or `ctrl+j` scrolls exactly once and
every further line or page needs a fresh keypress.

Key repeat is gated on `App::terminal_input_context()`, which yields a context
for `Terminal` and `Copy` and `None` for everything else. A `None` context
makes `InputLeaseTable::plan_repeat` return `RepeatPlan::Ignore`, so every
`KeyEventKind::Repeat` for the held key is dropped. The new mode was added
without a context, so it silently inherited the modal-mode behaviour that
exists to stop a held confirm key firing twice — the opposite of what a scroll
mode wants.

This is a gap in that change rather than a design choice: the mode was built
for exactly the repeat-heavy motion copy mode already supports, and the docs
shipped with it describe holding the keys to keep scrolling.

## What Changes

- `TerminalInputContext` gains an `AppScroll` variant, and
  `terminal_input_context()` returns it while the passthrough mode is active,
  so held keys repeat there as they do in copy mode.
- The variant is distinct from `Copy` rather than reused, so a transition
  between the two modes changes the context and stops the repeats, the same
  transition guard `Pane`, `Popup`, and `Copy` already rely on.
- The variant stays non-terminal-routing, so repeats re-dispatch through the
  app-level key path that already queues the passthrough sends, rather than
  being forwarded to the pane as raw keys.

## Impact

- Affected specs: `copy-mode-key-repeat` — `Held escape-coded keys repeat in
  copy mode` modified to admit the passthrough mode.
- Affected code: `src/app/mod.rs` (the enum variant and the context mapping).
  No change to the lease machinery, the passthrough handler, or any binding.
- **No wire changes**, no new config, no new keybinding. The fix is three lines
  plus tests; the repeat plumbing it hooks into is unchanged.
- Fork tracking: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/61.

## Non-goals

- Changing repeat behaviour for any other mode. Modal modes keep ignoring
  repeats, which is what stops a held confirm key from firing twice.
- Adding a repeat rate or acceleration control. The host terminal's own repeat
  rate drives this, as it does in terminal and copy modes.
