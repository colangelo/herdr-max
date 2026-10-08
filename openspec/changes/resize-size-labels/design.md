## Decisions (approved by ac, 2026-09-29)

1. **All panes of the visible tab.** With nested splits one border resizes many
   panes, so "the two beside the border" is not well defined; it is also what
   display panes shows.
2. **A passive layer, not a mode.** `Mode::DisplayPanes` takes the next key and
   click, which would end the drag or leave resize mode. The labels are drawn
   after the tab surface and before the mode overlay, from the same
   `display_panes_labels()`, with the index hidden.
3. **When:** `resize_labels_visible(now) = drag is a PaneSplit || mode ==
   Resize || now < resize_labels_until`. A drag step and its release set
   `resize_labels_until = now + 1 s`; a direct resize key sets it too. Any other
   key or mouse press clears it. The deadline joins the loops' wake-up list,
   like `display_panes_deadline`.
4. **Cost:** one boolean check per frame when idle; during a resize the labels
   for the visible tab only (a zoomed tab labels its one pane).

## Testing

- State: a split drag shows labels; release keeps them 1 s then hides; a key
  hides them; resize mode shows them until esc; a single resize key shows them
  1 s; labels carry sizes that follow a resize step.
- Render: labels drawn over resize mode keep the `RESIZE` bar; no `PANES`
  summary bar; hidden panes of other tabs are not labelled.
- `just bench-render-scale` 1 vs 15 panes, labels on.
