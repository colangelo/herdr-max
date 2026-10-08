# Visible todo ids

## Why

Todos have had stable per-pane ids since the store landed — the CLI addresses
them (`herdr todo done 12`), agents read them in `todo list --json` — but no
TUI surface shows them. So the user cannot tell an agent "do note 12" without
running the CLI first, and an agent handing work to another agent has an
address the human in the loop cannot see. Requested while dogfooding the
agent-notes convention, whose whole point is todos as shared references.

## What Changes

Rows on the panel and the board show the id dim and right-aligned as `#<id>`;
the edit overlay's title names the id it is editing; the board's content-width
measurement accounts for the id. The id shown is the id the CLI accepts —
nothing new is minted.

## Impact

- Affected capability: `pane-todos` (one added requirement)
- Affected code: the shared todo row renderer (`render_pane_todo_row`), the
  link chip's rect (one id-width narrower), the board's width measurement, the
  editor title; board rendered-layout snapshots re-record
- No server, API, protocol or config surface: the id already exists everywhere
  but the screen

## Non-goals

- A session-unique id. The pane-scoped id plus the pane is the address the CLI
  already speaks; inventing a second numbering to avoid saying the pane name
  would put two id systems on one todo.
- Showing the owning pane on every row. The panel is one pane, and the board's
  headings already name it.
