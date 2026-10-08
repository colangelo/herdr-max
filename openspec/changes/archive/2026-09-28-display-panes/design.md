## Decisions

**An overlay, not a toast.** It takes keys (dismiss, focus by number), so it
belongs in the `overlays!` list: `DisplayPanes(DisplayPanesState)`, mode
`DisplayPanes`, ASCII input. That gives it the mode/overlay invariant, the
exhaustive `overlay_help` entry and a rendered-layout test for free, the way the
kit expects. The retained PTY fast path already requires `Mode::Terminal`, so
the labels are never skipped by it.

**The deadline lives in the overlay state.** `DisplayPanesState { deadline }`.
Both loops (`App::next_loop_deadline_*` for the local and headless servers) add
`state.display_panes_deadline()` to their wake-up list, and both timer passes
call one `AppState::expire_display_panes(now)`. Replacing or closing the
overlay removes the deadline with it, so nothing can go stale. 3 seconds: long
enough to read two or three labels; any key closes it sooner.

**What is shown.** Per pane in `view.pane_infos` (so zoom shows only the zoomed
pane, and the current tab only): `N  <public id> · <name>  <cols>x<rows>`.
The name is `AppState::pane_display_label`, the one the navigator and the todo
board already use, so a pane is called the same thing everywhere; it is cut
first when the label does not fit. `cols`x`rows` is
`cols`x`rows` is the the pane's `inner_rect`, the size its program sees
(`tput cols`/`lines`). The outer rect from `herdr pane layout` can be larger by
the border and scrollbar gutter; the program's size is what matters for copied
text and menus. The label is a bordered panel (`render_panel_shell`) centred in
the pane, accent-coloured for the focused pane. A pane too small for the panel
gets a one-row label at its top-left, cut to fit. The mode bar
(`render_bottom_bar`, like resize mode) reads
` PANES  window WxH · panes WxH  1-9 focus  any key close`: the window is the
frame area, the panes figure is `view.terminal_area`.

**Numbering.** `N` is the pane's position in `view.pane_infos` plus one, which
is the layout's reading order. Only 1–9 are shown as focusable; tmux does the
same with its single-digit indices. A tenth pane and beyond still gets its
address and size, with `·` in place of a number.

**Keys.** A digit naming a shown pane focuses it (`focus_pane_internal_via_api`
at the App level, `focus_pane_in_workspace` in the state-only dispatcher) and
closes the overlay. Any other key closes it and is consumed, as in tmux.
