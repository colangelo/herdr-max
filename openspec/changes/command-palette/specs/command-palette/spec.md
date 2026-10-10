## ADDED Requirements

### Requirement: One key opens a command bar

herdr SHALL provide a `command_palette` action, bound to `prefix+:` by default and listed in the
help panel, that opens a bar listing herdr's commands. The bar SHALL close on `esc` with an empty
query, and on running a command.

#### Scenario: Opening the bar

- **WHEN** the user presses `prefix+:`
- **THEN** a bar opens with an empty query, the list of commands and the count of commands

#### Scenario: Closing the bar

- **WHEN** the bar is open with an empty query and the user presses `esc`
- **THEN** the bar closes and nothing runs

### Requirement: The bar lists every command the help panel lists

The bar's rows SHALL come from the same command table as the help panel, keeping the entries that
name an action, each with its group and its current key (or `unset`). A command added to the help
table SHALL appear in the bar without a second list.

#### Scenario: A rebound command

- **WHEN** the user binds `split_vertical` to `prefix+|` and opens the bar
- **THEN** the `split vertical` row shows `prefix+|`

#### Scenario: An unbound command

- **WHEN** an action has no key
- **THEN** its row shows `unset` and running it from the bar still works

### Requirement: Typing filters the list

Typing SHALL filter the rows by a fuzzy subsequence match over the group and the label, best match
first, help order on ties. The count SHALL read `<shown> of <total>` while a query is set.

#### Scenario: A short query

- **WHEN** the user types `spl`
- **THEN** `split vertical` and `split horizontal` are listed before rows that match less closely

### Requirement: Enter runs the selected command as its key would

`enter` SHALL close the bar and run the selected row's action through the same path a key press
uses, so the command behaves exactly as when its key is pressed.

#### Scenario: Running a command that opens a prompt

- **WHEN** the user runs `rename tab` from the bar
- **THEN** the rename prompt opens, as with the rename key

### Requirement: The bar follows the style guide

The bar SHALL follow `docs/ui-style.md`: the title in bold with the count dim on the right, the
query as the subtitle, a rule, the selected row as the accent bar, shortcuts dim and right-aligned,
and a key bar footer.

#### Scenario: Rendered layout

- **WHEN** the bar renders at 82 columns with three matching rows
- **THEN** the rendered-layout test finds the title row, the query row, the rule, one accent bar
  and the key bar in that order
