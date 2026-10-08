## ADDED Requirements

### Requirement: A list dialog's status column is measured, not stepped

A centred list dialog with a right-hand status column (the session navigator,
the move-pane picker) SHALL measure that column from its rows, counting the
longest state word a row can show, up to a per-dialog cap, and SHALL draw the
column at the width it measured. When the dialog is narrower than its measured
width, the label column SHALL give way first, down to a floor, before any
status is cut.

#### Scenario: A status fits the box it sized

- **WHEN** the navigator opens in a 310x56 window on rows whose widest status
  is `keyboard-shortcuts · idle`
- **THEN** every status is drawn whole, and the box is no wider than its
  labels, that status column and its border need (never under its footer
  floor, never over 120)

#### Scenario: A state change does not overflow the column

- **WHEN** a pane's status changes from `idle` to `working` while the
  navigator is open
- **THEN** the new status still fits the column, and the box does not change
  width

#### Scenario: A small window cuts labels before statuses

- **WHEN** the navigator opens in a window narrower than its measured width
- **THEN** long labels are cut first, and a status is cut only once the label
  column is at its floor
