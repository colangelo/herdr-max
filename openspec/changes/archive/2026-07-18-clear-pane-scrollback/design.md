## Context

Herdr embeds the vendored Ghostty emulator; a pane's scrollback lives inside that emulator (constructed with a byte budget via `crate::ghostty::Terminal::new(cols, rows, max_scrollback)` in `src/ghostty/mod.rs`, budget `advanced.scrollback_limit_bytes`, default 10 MB). Incoming PTY bytes are forwarded to the emulator in `src/pane/terminal.rs`. The emulator honors `CSI 3J` (`ESC[3J`, ED3 "erase saved lines") internally by purging its scrollback. Herdr detects that sequence with `contains_scrollback_clear_sequence` (`src/pane/osc.rs:802`) and, only for the `droid` foreground job on the primary screen, strips it (`maybe_filter_primary_screen_scrollback_clear`, `strip_scrollback_clear_sequences`, `src/pane/osc.rs:807-847`) — a scoped compatibility hack whose own comment notes normal clear-history behavior must keep working elsewhere.

Today there is no herdr-originated clear: no CLI subcommand, no `Method`/`Request` variant, no `NavigateAction`. `Ctrl-L`/redraw-`clear` emit only `ESC[2J` (visible screen), leaving scrollback intact. There is an unused `ghostty_terminal_reset` FFI (`src/ghostty/bindings.rs:1157`) that performs a full RIS reset (screen, modes, scroll region, and scrollback) — too broad for a clear-history action and never wrapped.

This is a shared runtime/terminal fact (the pane's buffer), so per the boundary guardrail it should be exposed through the JSON API/CLI as well as the TUI, under a neutral `pane.clear` name.

## Goals / Non-Goals

**Goals:**
- Provide a first-class, scrollback-only purge with tmux `clear-history` semantics.
- Expose it on three surfaces: keybinding (`ClearScrollback`), socket method (`pane.clear`), CLI (`herdr pane clear`).
- Reuse the existing pane write path so the emulator does the actual purge exactly as it does for program-emitted `CSI 3J`.
- Keep the visible screen and running process untouched.
- Ensure the herdr-originated clear is not suppressed by the droid passthrough filter.

**Non-Goals:**
- Full terminal reset (RIS). The `ghostty_terminal_reset` FFI stays unused.
- Any change to `scrollback_limit_bytes` or scrollback storage/budget.
- Changing or removing the existing droid `CSI 3J` strip behavior for program-emitted sequences.
- A "clear visible screen" action (that is already achievable by the program / `Ctrl-L`).

## Decisions

### Decision 1: Inject `CSI 3J` through the write path, not RIS

Implement the purge by writing `ESC[3J` into the pane's terminal via the existing write path (the same route program bytes take), so the emulator purges its own scrollback with no new FFI. Rationale: it reuses a proven code path, matches tmux `clear-history` (scrollback only), and avoids the collateral of RIS (wiping screen contents/modes/scroll region), which would surprise users and disrupt a running full-screen app.

Alternative considered: wrap and call `ghostty_terminal_reset` (RIS). Rejected — semantically wrong for "clear history" and destructive to visible state.

Alternative considered: add a dedicated scrollback-only clear FFI. Rejected for v1 — unnecessary when `CSI 3J` already does exactly this through the existing path; can revisit if the emulator API changes.

### Decision 2: Add a `pane.clear` socket method and route all surfaces through it

Add `Method::PaneClearScrollback` (`pane.clear`) in `src/api/schema.rs` with a handler in `src/app/api/panes.rs` that resolves the target pane and triggers the injection on its terminal. The CLI `herdr pane clear` sends this request; the TUI `ClearScrollback` action calls the same client path for the focused pane. Rationale: single runtime entry point, consistent behavior across surfaces, and the terminal buffer is a shared fact that belongs in the API per the boundary guardrail.

Protocol handling: this is additive. Per project rule, compare `PROTOCOL_VERSION` against the latest released tag and bump only if the source protocol is not already ahead; update hardcoded protocol expectations/fixtures if bumped.

Alternative considered: TUI-only clear that pokes the terminal directly, no API. Rejected — would hide a runtime capability from non-TUI clients and violate the guardrail.

### Decision 3: Bypass the droid strip for herdr-originated clears

The droid `CSI 3J` filter targets bytes coming *from the program* on the PTY read path. The herdr-originated clear must reach the emulator regardless. Implement by injecting at a point after (or otherwise not subject to) the droid strip, or by an internal call that goes straight to the emulator's clear rather than through the filtered program-byte path. Rationale: the filter exists to suppress droid's *own* clears, not the user's explicit request; the spec requires the action to work even when droid is foreground.

### Decision 4: Keybinding default deferred; ship the action + config key regardless

Register a `ClearScrollback` `NavigateAction` and a `KeysConfig` field next to `edit_scrollback` (`prefix+e`). Whether to bind it by default is an open question (see below): the crowded prefix space and the adjacency to `edit_scrollback` argue for either a nearby chord or shipping unbound. The action and config key exist either way so users can bind it.

## Risks / Trade-offs

- **Droid filter interaction** → if the injection point is chosen wrong, the clear could be stripped when droid is foreground (the exact case the spec calls out). Mitigation: inject on a path not subject to the droid strip, with a test that clears succeed under a simulated droid-foreground state.
- **Alt-screen behavior** → `CSI 3J` on the alternate screen may behave differently (alt screens typically have no scrollback). Mitigation: treat as a safe no-op on alt screen; the visible screen must remain untouched. Covered by the empty-scrollback no-op scenario.
- **Protocol version churn** → an unnecessary bump breaks fixtures. Mitigation: follow the "bump only if not already ahead" rule and update fixtures if bumped.
- **User expectation vs. RIS** → some users may expect `clear` to also wipe the screen. Mitigation: document that this is scrollback-only (tmux `clear-history`), distinct from `Ctrl-L`.

## Migration Plan

Additive across CLI, API, and TUI; no persisted state or storage format changes. If `PROTOCOL_VERSION` is bumped, update hardcoded expectations and manual protocol fixtures in the same change. The new `KeysConfig` field defaults per the resolved open question; existing configs inherit the default. Rollback is reverting the code and (if bumped) the protocol constant. Docs land in `docs/next/` (`keyboard.mdx`, `cli-reference.mdx`, `socket-api.mdx`) until release.

## Open Questions

- Default keybinding for `ClearScrollback`: ship bound (a chord near `prefix+e`) or unbound-by-default. The prefix space is crowded, so unbound is a defensible default with easy opt-in.
- Whether `pane.clear` should also emit a layout/pane-updated event or a dedicated `pane.cleared` event, or simply return success (leaning: success response only, since geometry does not change).
- Exact injection point relative to the droid strip — pick the seam that guarantees the herdr-originated clear reaches the emulator while leaving the droid program-byte filter intact.
