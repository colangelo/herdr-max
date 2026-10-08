## Why

Herdr has no first-class way to clear a pane's saved scrollback, unlike tmux's `clear-history`. Today the buffer is only purged passively, when a program inside the pane emits `CSI 3J` (e.g. `clear` on an `E3`-capable `TERM`, or `printf '\e[3J'`). `Ctrl-L` and a plain redraw-`clear` only erase the visible screen and leave scrollback intact. Users who want to drop a large or stale scrollback (for privacy, to reclaim the buffer budget, or just to tidy a noisy agent pane) have no direct herdr command, keybinding, or API to do so.

## What Changes

- Add a **clear-scrollback** action that purges only the focused pane's saved scrollback (visible screen and running process untouched), by injecting `CSI 3J` (`ESC[3J`) through the existing pane write path into the embedded Ghostty emulator — the same mechanism programs already use, so semantics match tmux `clear-history` rather than a full terminal reset.
- Expose it on three surfaces:
  - **Keybinding**: a new `NavigateAction::ClearScrollback` with a configurable `KeysConfig` entry and a default binding (to be finalized in design; unbound-by-default is an option given `prefix+e` `edit_scrollback` already sits nearby).
  - **Socket API**: a new `pane.clear` method that clears the scrollback of a target pane.
  - **CLI**: `herdr pane clear [<pane_id>|--current]` that sends the `pane.clear` request.
- Deliberately **not** use the full `ghostty_terminal_reset` (RIS) FFI: that would also wipe screen contents, modes, and the scroll region, which is a harder reset than "clear history" and not what users expect from this action.
- Preserve the existing droid-compat behavior that strips `CSI 3J` from the droid foreground job on the primary screen; the new action is an explicit herdr-originated clear and should not be affected by that passthrough filter.

## Capabilities

### New Capabilities

- `pane-clear-scrollback`: The ability to purge a pane's saved scrollback buffer on demand via keybinding, socket method (`pane.clear`), and CLI (`herdr pane clear`), leaving the visible screen and process state intact. Covers the scrollback-only semantics, the three surfaces, target-pane resolution, and interaction with the alternate screen and the droid passthrough filter.

### Modified Capabilities

<!-- None. No existing OpenSpec specs; this adds a new protocol method rather than changing an existing spec's requirements. -->

## Impact

- **Terminal/emulator layer**: a scrollback-clear method on the Ghostty `Terminal` wrapper (`src/ghostty/mod.rs`) or the pane runtime (`src/pane/terminal.rs`), injecting `CSI 3J` through the existing write path. The unused `ghostty_terminal_reset` FFI is explicitly not adopted.
- **Protocol/API**: a new `Method::PaneClearScrollback` (`pane.clear`) variant in `src/api/schema.rs` with a handler in `src/app/api/panes.rs`. This is an additive protocol method — check `PROTOCOL_VERSION` against the latest released tag and bump only if not already ahead.
- **TUI input layer**: new `NavigateAction::ClearScrollback` + dispatch in `src/app/input/navigate.rs`; new `KeysConfig` field/default in `src/config/model.rs` and the keymap table.
- **CLI**: new `clear` subcommand under `herdr pane` in `src/cli/pane.rs` / `src/cli/spec.rs`.
- **Docs**: `keyboard.mdx`, `cli-reference.mdx`, and `socket-api.mdx` (staged under `docs/next/` until release).
- **Boundary guardrail**: the scrollback purge is a shared runtime/terminal fact, correctly exposed through the JSON API/CLI as well as the TUI, using a neutral `pane.clear` name.
