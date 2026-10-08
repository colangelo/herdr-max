## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/141, asked by ac
2026-09-30 ("is there in herdr a sync panes mode, where i type in every pane
at the same time?"), design confirmed 2026-10-02 (issue comment 18949). herdr
has no synchronized-input or broadcast mode; the nearest thing is scripting
`pane send-text` per pane, which is a one-shot send, not live typing. tmux's
`synchronize-panes` is the model.

## What Changes

- **A sync mode per tab.** A key turns it on and off for the current tab. While
  it is on, what is typed into the focused pane is also sent to every other pane
  of that tab. Every pane of the tab is in the set when it is turned on, agent
  panes included (ac chose "all panes").
- **Per-pane opt-out by right click.** While sync is on, a right click on a pane
  takes it out of the set and a right click on it again puts it back. This also
  works on any pane, whichever pane holds the cursor.
- **A bright yellow hint.** While sync is on, the borders of the synced panes and
  the bottom bar turn bright yellow, the way the labels view turns them red on
  `prefix+i`, a resize or a sidebar drag. A pane taken out of the set drops the
  yellow, so the set is readable at a glance.
- **Server-side state.** Input fan-out is a runtime fact, so the mode lives with
  the tab in the server, not in a client, and every attached client shows the
  same yellow and gets the same fan-out.
- **API and CLI.** `tab.sync` (`herdr tab sync on|off|toggle`) and `pane.sync`
  (`herdr pane sync on|off|toggle`) set the mode and a pane's membership;
  `sync` on tab info and `synced` on pane info (optional fields).
- **Key.** Action `toggle_sync_panes`, default `prefix+shift+s`, with a help entry.

## Capabilities

### New Capabilities

- `pane-sync`: typing into several panes of a tab at once.

## Impact

`src/workspace.rs` (per-tab sync state), `src/app/input/{terminal,mod}.rs` (the
fan-out), `src/app/input/mouse.rs` and `src/app/state.rs` (right click and the
pane menu), `src/ui/{panes,display_panes}.rs` (yellow borders and bar),
`src/api/schema/*` (two methods, optional fields), `src/cli/*`,
`src/config/keybinds.rs`, `src/ui/keybind_help.rs`, docs. `PROTOCOL_VERSION` is
not bumped: only new methods and optional response fields.

## Open points for ac (each is one constant or one default)

1. The sync key: `prefix+shift+s` (decided by ac, comment 18977).
2. Whether the mode survives a restart or a live handoff: **no** (decided by
   ac). A mode that types into several panes must not come back on its own.
3. Whether an agent pane that is working is skipped by default. Default: **no**
   (ac said all panes), but see Risks.
