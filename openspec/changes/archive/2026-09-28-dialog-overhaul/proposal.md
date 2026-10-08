## Why

On 2026-09-28 ac asked that "all the dialog panes i mentioned need a visual
overhaul, to make them more intuitive and uniform with the most advanced parts
of the interface". The four dialogs he named share almost nothing today. Each
places itself its own way, and only the navigator can be searched. One of them
also shows a wrong name.

- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/103: after a pane is
  moved to a new space, the move-pane dialog lists that space under the name of
  the space the pane came from ("master" twice).
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/104: the move-pane
  dialog has no `/` search.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/105: the session
  navigator is 272 columns wide in a 310x56 window.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/106: the pane-todo
  dropdown shows the selected todo's full text in some spaces and not others,
  and when it does, the text is hard to tell apart from the list.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/107: the todo board
  has no `/` search.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/108: umbrella; build
  it on the overlay kit, with one shared search piece rather than copies.

## What Changes

- **One search piece in the overlay kit** (`ListSearch`: query field, focus,
  word matching, key handling, search row). The navigator moves onto it with no
  visible change. The move-pane picker and the todo board gain `/` search with
  the navigator's keys and look.
- **One placement and width rule for centred list dialogs.**
  `AnchoredPanelSpec` gains a centred placement. The navigator, move-pane
  picker and todo board resolve through it, each measuring its own content,
  with a shared maximum width (120). The navigator stops filling the screen.
- **A boxed detail for the selected todo.** It shows whenever the selected
  todo has text the row cannot show (more than one line, or a cut first line).
  It sits in its own inner box, and its height does not jump as the selection
  moves. The todo board gets the same box.
- **The move-pane picker names spaces the way the sidebar does** (the live
  root-pane cwd, not the stale stored cwd). A pane moved to a new space seeds
  that space's identity from its live cwd.
- The move-pane picker moves onto `ButtonRow` and `ListCursor` windowing and
  gains a rendered-layout test. It is the only one of the four without these.

## Impact

- TUI presentation only, plus one naming fix. No API field, event, protocol or
  config change. No new keybinding: `/` works inside the dialogs, as it does in
  the navigator.
- Code: `src/ui/overlay/` (new `search.rs`, `geometry.rs`), `src/app/input/`
  (navigate, modal, mouse, overlays), `src/ui/{navigator,dialogs,todo_panel,todo_board}.rs`,
  `src/app/state.rs`, `src/app/actions.rs`, `src/app/api/panes.rs`
  (new-space identity).
- Docs: `docs/next/website/src/content/docs/keyboard.mdx`.
- Built in one batch with JOB 8
  (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/102,
  `openspec/changes/detached-size-remembers-client`).
