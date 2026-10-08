## Why

Held modified shortcuts do not repeat outside terminal passthrough. In copy mode,
holding `h`/`j`/`k`/`l` keeps moving the cursor (each held key arrives as repeated
literal text bytes, re-parsed as fresh `Press` events), but holding `Ctrl-U` /
`Ctrl-D` (half-page scroll) fires only once. The same is true for every
escape-coded shortcut across herdr's own action modes (Copy, Navigate, Prefix,
dialogs): the outer terminal faithfully emits `KeyEventKind::Repeat` events
(herdr requests the Kitty keyboard protocol with `REPORT_EVENT_TYPES`), but both
input consumers drop them unless `mode == Mode::Terminal` or a popup is open.

That Terminal-only gate is intentional — it stops a held modal confirm/close key
(e.g. `Enter` on a release-notes dialog) from repeating into the shell (see the
`0.7.4` "held-key repeat in terminal panes" changelog note and the
`suppressed_repeat_keys` set). But it was never extended to herdr's own
repeatable motions, so legitimate held actions like copy-mode paging are dropped.

The gate lives at two consumer sites (live `runtime.rs`, headless `mod.rs`) and is
enforced twice per site: once at `Repeat`-dispatch time, and once at `Press` time
where the key is added to `suppressed_repeat_keys`. Widening only the dispatch
gate is not enough — the suppression set still blocks the repeat.

## What Changes

- Add a `Mode::honors_key_repeat()` helper (true for `Terminal | Copy`) that
  names the set of modes in which held escape-coded keys should re-dispatch.
- Use the helper consistently at all four spots — the `Repeat`-dispatch branches
  and the `Press`-time `suppressed_repeat_keys` inserts, in both the live
  (`src/app/runtime.rs`) and headless (`src/app/mod.rs`) input paths — so copy
  mode honors key repeat while the modal-leak protection is preserved for every
  other non-terminal mode.
- In the headless path, decouple the repeat decision from the terminal-vs-non-
  terminal handler routing so a repeat in copy mode is routed to the non-terminal
  (copy-mode) handler, not the terminal handler.
- Add repeatable `Ctrl-K` / `Ctrl-J` viewport scroll in copy mode: `Ctrl-K`
  scrolls the window up through scrollback (revealing older lines) and `Ctrl-J`
  scrolls it back toward the live bottom, both **without moving the cursor**
  relative to the buffer text (vim `Ctrl-Y` / `Ctrl-E` semantics), until the
  cursor would leave the viewport, where it sticks to the edge.
- Surface the new `Ctrl-K` / `Ctrl-J` keys in the copy-mode overlay footer so
  they are discoverable, and document them (plus the now-working held-key repeat)
  in the keyboard reference.
- Non-goals: a tmux `repeat-time` / `-r` style repeat window, making repeat
  configurable, enabling repeat in Prefix / Navigate / dialog modes, or touching
  the pane passthrough gate (`src/pane/terminal.rs`).

## Capabilities

### New Capabilities

- `copy-mode-key-repeat`: Held escape-coded shortcuts re-dispatch in copy mode
  (and terminal mode) so copy-mode paging/scroll motions repeat while held,
  without letting modal confirm/close keys repeat into the shell; plus repeatable
  `Ctrl-K` / `Ctrl-J` line-wise viewport scroll in copy mode.

### Modified Capabilities

<!-- None. There is no pre-existing OpenSpec spec for the input/key-repeat path. -->

## Impact

- **Mode model** (`src/app/state.rs`): new `Mode::honors_key_repeat()` predicate.
- **Live input** (`src/app/runtime.rs`): `Press` suppression insert and `Repeat`
  dispatch now gate on `honors_key_repeat()` instead of `== Mode::Terminal`.
- **Headless input** (`src/app/mod.rs::route_client_events`): same gate change,
  plus the `Repeat` branch routes to the correct (terminal vs non-terminal)
  handler for the active mode.
- **Copy mode** (`src/app/input/copy_mode.rs`): new
  `scroll_copy_mode_viewport_line` + `Ctrl-K` / `Ctrl-J` handlers.
- **UI** (`src/ui/menus.rs`): copy-mode overlay footer gains a scroll hint.
- **Docs** (`docs/next/website/src/content/docs/keyboard.mdx`,
  `docs/next/CHANGELOG.md`): document the fix and the new keys.
- **Runtime/protocol**: no new socket message, no protocol change, no server
  state. This is TUI-client input handling and shared mode semantics.
- **Boundary guardrail**: no new shared-runtime surface; the change is input
  projection in the client, named neutrally (key repeat / viewport scroll).
- **Upstreamable**: the repeat-drop bug exists upstream too; this fix is generic.
  Tracked on the fork only — do not open anything upstream.
