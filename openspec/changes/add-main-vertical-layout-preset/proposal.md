## Why

ac requested a focused main pane on the left and stacked panes on the right (2026-10-12). The three existing presets cannot produce this geometry.

Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/215

## What Changes

- Add main-vertical after tiled in the layout cycle, with the focused pane taking the left half.
- Stack one through four other panes on the right; use two columns for five or more, filled top-down in current pane order.
- Add layout.apply_preset without modifying the frozen layout.set_preset contract.
- Gate the client cycle by advertised capability and reuse transient feedback after successful application.
- Accept --preset main-vertical and main-v in the CLI, retaining --set.

## Capabilities

### New Capabilities
- `layout-presets`: focused main-vertical geometry and compatible preset application.

## Impact

Core layout, public API/schema, endpoint advertisement and shape tests, client cycle and feedback, CLI, staged documentation. No published codec or frozen method shape changes.
