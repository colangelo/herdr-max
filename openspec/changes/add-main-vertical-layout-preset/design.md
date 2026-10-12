## Context

The approved design is bounded additive work. Shared pane-tree geometry belongs to the runtime; cycle cursor and feedback belong to the client. Protected behavior: pane identity, focus/history, original three preset geometry, endpoint method shapes and old-server cycle.

## Decisions

MainVertical rebuilds once per explicit action (O(panes)); no render-loop or detection work is added. Root is a horizontal 0.5 split whose first child is the focused pane. Remove that pane from current pane_ids order to construct the right subtree. Up to four others form an equal-height vertical chain. Five or more split horizontally at 0.5, with ceil(n/2) others in the first vertical chain and the rest in the second. One pane is a no-op.

Keep API LayoutPreset and LayoutSetPresetParams immutable. Add LayoutApplyPresetParams and AppliedLayoutPreset for all four values to layout.apply_preset; reuse LayoutPresetApplied response. Advertise and freeze the additive method independently of the generation-1 fixture. Include it in mutation/refresh classification.

The client uses layout.set_preset for old presets. It includes main-vertical only when layout.apply_preset is explicitly advertised. Success feedback uses the existing transient toast through a pending response kind, so rejected applications do not claim success.

CLI retains --set and adds --preset, dispatching only main-vertical through the additive method. Staged docs describe exact geometry and compatibility.

## Validation

Test-first slices at the approved core, handler, frozen-schema, client-cycle and CLI seams. Core worked examples cover 2, 3, 4, 5, 6 and 9 panes at 290x54, focused pane not first. Characterize single-pane no-op and focus history. Final locked offline just check and pinned-toolchain clippy; no network or runtime deployment.
