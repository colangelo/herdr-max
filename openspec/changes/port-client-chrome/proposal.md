## Why

The sync to upstream v0.9.3 stripped fork presentation code from the server while retaining shared facts. Restore the running fork's chrome in the client shell, with the same glyphs, colors, positions, timings, defaults and mouse/key behavior, as authorized by ac through herdr-sync in fork issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/171.

## What Changes

- Restore client display ranking, sidebar state presentation, editorial/jump/border layout, custom token fitting, per-endpoint pins/order, focus-follow, overflow fog/fade, and working/background animation.
- Project shared runtime facts optionally through existing endpoint snapshot JSON without changing frozen generation-one binary codecs or existing method meanings.
- Adapt live server pane borders/tint/dim/todo indicators; rebuild client labels, pane-relative feedback, toast size/duration and tab notification/todo indicators.
- Preserve existing sound/hint service behavior and completion evidence. Coordinate input and overlay seats through one fact projection; leave overlays themselves to their owner.
- Keep animation and exact fork wording separately reviewable as fork-only commits.

## Capabilities

### New Capabilities

- `session-resource-projection`: optional version-tolerant runtime facts accompanying endpoint JSON snapshots, with stable resource identities and core-codec compatibility.

### Modified Capabilities

None. Existing sidebar-agent-state-display, sidebar-editorial-style, sidebar-number-prefix, sidebar-sort-motion, collapsed-sidebar-parity, pane-todos, notification-center and display-panes behavior is preserved by moving its presentation to the client; the original fork commits and specs remain the parity oracle.

## Impact

src/client/shell, src/client/endpoint, src/protocol/endpoint.rs, src/server/client_shell.rs, live src/ui helpers, config reload and discoverable help; optional endpoint JSON facts. No dependency additions or release/production-doc changes. Plan committed before code; each feature has its own commit and progress record. Full tests await lead's test-clean merge tag.
