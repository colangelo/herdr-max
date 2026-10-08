## 1. Terminal-level clear

- [x] 1.1 Add a scrollback-clear method on the pane runtime / Ghostty `Terminal` wrapper (`src/pane/terminal.rs` near `scroll_reset`, or `src/ghostty/mod.rs`) that injects `ESC[3J` through the existing write path
- [x] 1.2 Ensure the injection is not subject to the droid `CSI 3J` strip (`src/pane/osc.rs`), so a herdr-originated clear reaches the emulator even when `droid` is the foreground job — done by construction: `clear_scrollback` feeds the emulator directly, bypassing `process_pty_bytes` where the strip lives
- [x] 1.3 Confirm the visible screen and process are untouched, and that the call is a safe no-op on empty scrollback / alt screen

## 2. Socket API

- [x] 2.1 Add `Method::PaneClearScrollback` (`pane.clear`) variant in `src/api/schema.rs`
- [x] 2.2 Add the handler in `src/app/api/panes.rs` that resolves the target pane id and triggers the terminal-level clear; return a not-found error for unknown pane ids
- [x] 2.3 Compare `src/protocol/wire.rs::PROTOCOL_VERSION` against the latest released tag; bump only if not already ahead, and update hardcoded protocol expectations/fixtures if bumped — no bump: source is 18, latest released tag v0.7.4 shipped 16, already ahead

## 3. TUI keybinding

- [x] 3.1 Add `NavigateAction::ClearScrollback` in `src/app/input/navigate.rs` and dispatch it to the `pane.clear` client path for the focused pane
- [x] 3.2 Add a `clear_scrollback` field to `KeysConfig` (`src/config/model.rs`) next to `edit_scrollback`; resolve default-bound-vs-unbound per design Open Questions and wire it in the keymap table — resolved unbound-by-default per the house convention "optional actions are unset by default"

## 4. CLI

- [x] 4.1 Add a `clear` subcommand under `herdr pane` in `src/cli/pane.rs` / `src/cli/spec.rs` accepting an explicit pane id and `--current`
- [x] 4.2 Send the `pane.clear` request over the session socket and report success/failure

## 5. Tests

- [x] 5.1 Unit test the terminal-level clear: scrollback emptied, visible screen preserved, no-op on empty scrollback
- [x] 5.2 Test that the herdr-originated clear succeeds under a simulated `droid` foreground state (not suppressed by the passthrough filter) — the test asserts the program-byte filter strips droid-emitted `3J` while the direct-injection clear still purges
- [x] 5.3 Test the `pane.clear` handler: success for a valid pane, not-found for an unknown pane id
- [x] 5.4 Add a keymap test for the `clear_scrollback` config field mapping to `NavigateAction::ClearScrollback`; if the protocol was bumped, update protocol fixtures — no bump, no fixture changes

## 6. Docs and validation

- [x] 6.1 Document `pane.clear` in `docs/next/website/src/content/docs/socket-api.mdx`, `herdr pane clear` in `cli-reference.mdx`, and the keybinding (if bound) in `keyboard.mdx` / `configuration.mdx` — socket-api + cli-reference + config reference JSON + CHANGELOG updated; keyboard.mdx intentionally untouched (unbound-by-default actions are not listed there, matching `last_pane`)
- [x] 6.2 Run `just check` and fix any failures — green after regenerating the API schema artifact (`HERDR_UPDATE_API_SCHEMA=1`)
- [x] 6.3 Dogfood live: fill a pane's scrollback, run the clear via keybinding, `herdr pane clear`, and the socket method; confirm scrollback is gone while the visible screen and process remain — verified 2026-07-18 against a live debug server: 183 scrollback lines → `herdr pane clear` (the CLI drives the `pane.clear` socket method) → `max_offset_from_bottom` 0, visible tail byte-identical, shell still responsive; the keybinding dispatches this same `pane.clear` path and is covered by the keymap unit test (live key press verification rides the next beta, since the installed herdr-beta predates this build)
