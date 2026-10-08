## ADDED Requirements

### Requirement: Pane sizes show while a pane is resized

While a split border is dragged, while keyboard resize mode is on, and for 1
second after a drag ends or a single resize key is pressed, every pane of the
visible tab SHALL show its display-panes label without the index, with its
current size. The labels SHALL NOT take input, and any other key or mouse
press SHALL hide them.

#### Scenario: Dragging a border

- **WHEN** the user drags the border between two panes
- **THEN** every pane of the tab shows `<address> · <name>  <cols>x<rows>`, the
  sizes change with each drag step, and the labels go 1 second after release

#### Scenario: Keyboard resize mode

- **WHEN** the user enters resize mode and presses `l` twice
- **THEN** the labels show the new sizes after each press, the `RESIZE` bar
  stays, and esc hides the labels with the mode
