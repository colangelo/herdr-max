# tui-overlay-kit

## ADDED Requirements

### Requirement: Titled overlays reserve a header block

An overlay that draws a title SHALL reserve a header block of that title plus
one blank row beneath it, defined once in the kit as the mirror of the footer
block, so a title is never drawn flush against the overlay's first content row.

An overlay whose header is more than one line SHALL extend the block by those
lines rather than redefining it, and the extended value SHALL be named once and
read by both that overlay's renderer and its hit-test, so what is drawn and what
is clickable cannot disagree about where the content starts.

An overlay that draws no title reserves no header block.

#### Scenario: A title is not drawn against its content

- **WHEN** a titled overlay is rendered
- **THEN** a blank row separates its title from its first content row

#### Scenario: Render and hit-test agree on where rows begin

- **WHEN** a titled overlay's row is clicked
- **THEN** the row acted on is the row drawn at that cell

#### Scenario: An untitled overlay is unaffected

- **WHEN** an overlay with no title is rendered
- **THEN** its content begins at its first inner row
