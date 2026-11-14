# Mark unread agents read in bulk

Tracking issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/276

## Why

An agent that finishes while its tab is not shown stays unread (`seen = false`): the
agent sidebar shows it as `done` and its space carries the unread mark. The only way
to clear it today is to focus its tab. With a fleet of agents across many spaces, ac
wants to clear them all at once from a right-click menu (ac, 2026-11-14).

## What changes

- New server API method **`pane.mark_seen`**: marks every unread pane read, either in
  the whole session or in one space (`workspace_id`). Returns how many changed.
- CLI: `herdr pane mark-seen [--workspace <id>]`.
- Client right-click menus:
  - space row → **Mark space read** (that space) and **Mark all read** (session);
  - agent sidebar row → **Mark all read**.
  Items appear only when something they would clear is unread, and only when the
  connected server advertises `pane.mark_seen`.

## Why it is portable

The state change lives on the server, behind a neutral, advertised API method, so the
same action works from a local, SSH or remote client, from scripts and the CLI, and
maps onto a server-owned fact when herdr-max becomes a herdr-core client. The menu is
presentation only. A client connected to an older server simply hides the items
(AGENTS.md § Stable client endpoint contract). The change is shaped to be PR-able
upstream.

## Out of scope

- Notification-center entries (`notification.mark_seen` already clears those).
- A keybinding (can follow; it would need a `keybind_help.rs` entry).
