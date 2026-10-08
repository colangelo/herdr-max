# Proposal

## Why

The v0.9.3 merge retains Herdr Max runtime facts and configuration but removes the server TUI input handlers. Daily-use fork keys, copy/SCROLL gestures and mouse controls must work again through the client-owned shell, matching the frozen fork at `sync-snapshot/2026-10-08`.

Approved by ac through lead `herdr-sync`; tracked at https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/171. Assigned route: `r-1008-07`.

## What Changes

- Connect retained letter bindings, keybind warnings and action configuration to client input and discoverable help.
- Restore copy viewport motions and one-gesture entry, plus alternate-screen SCROLL with target-safe repeats and application-specific top/bottom keys.
- Connect layout, respawn, scrollback-only clear, pane-to-tab, pin and sync controls to advertised runtime APIs.
- Keep sync fanout limited to typed keys/text/paste; use a separately advertised scroll-intent method for generated SCROLL input.
- Integrate client mouse controls with chrome-owned geometry/presentation; leave notification/todo/move overlay implementations to the overlays seat.
- Preserve upstream multi-prefix behavior, input leases, pane-move reconciliation and generation-1 compatibility. Record any architecture-forced visible differences in the assigned progress record.

## Capabilities

### New Capabilities

- `endpoint-input-routing`: client ownership of commands and modes, advertised runtime mutations, target-safe typed and generated application input across compatible endpoints.

### Modified Capabilities

None. Existing fork copy, clear, move, overlay and runtime requirements remain authoritative; this port restores their implementation in the new architecture.

## Impact

`src/client/shell/`, `src/input/keybindings.rs`, `src/input/keybind_help.rs`, API advertisement/schema/dispatch and semantic pane input. Existing server group/pin/layout/lifecycle facts are reused. No new dependency; no mutation of frozen generation-1 codecs or existing method semantics. Work occurs only on `port/input`; no push/merge/cutover.
