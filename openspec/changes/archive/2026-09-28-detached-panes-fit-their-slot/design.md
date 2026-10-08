## Decisions

**Follow upstream's detached size.** A first attempt made the last client's size
the no-client size and resized panes on every no-client frame. That contradicts
upstream's configurable headless size (#2829, test
`pane_created_after_detach_uses_configured_headless_size`: panes created after a
detach use the configured size, existing panes keep theirs), so it was dropped.

**State flag.** The headless server sets `AppState::detached_pane_size` to its
no-client size whenever no foreground client is attached, and clears it when one
attaches. `estimate_pane_size()` returns it when set.

**Splits.** `App::split_sizes` returns the estimate when a client is attached.
Otherwise it returns the target's current size split by `split_shares` (the kept
share is the left or top part, as in the layout), plus the kept share for
`resize_split_target`. It is used by the API split and by layout apply; key and
plugin splits happen with a client attached.

## Known limit

A detached pane's width is the configured headless width, not its slot's (the
sidebar is not subtracted), as upstream specifies. Rows follow the split ratio.
