# pane-send-text-chunking

## ADDED Requirements

### Requirement: `pane send-text` can send text in paced pieces

`herdr pane send-text` SHALL accept `--chunk <BYTES>`. With it, the CLI SHALL
split the text into pieces of at most BYTES bytes and send each piece as a
separate `pane.send_text` request, pausing `--chunk-delay <MS>` milliseconds
(default 20) between pieces. The command SHALL return only after the last piece
has been sent, and SHALL stop and report how many pieces were sent if one
request fails.

Without `--chunk`, the command SHALL send the text as one request, unchanged.
`--chunk-delay` without `--chunk` SHALL be rejected. `--chunk 0` SHALL be
rejected. `--` SHALL end option parsing.

#### Scenario: Large text reaches Claude Code as typed text

- **WHEN** 3000 bytes are sent to an idle Claude Code input with `--chunk 300`
- **THEN** the input box holds the exact text
- **AND** no `[Pasted text` placeholder appears

#### Scenario: Unchunked send is unchanged

- **WHEN** `pane send-text <pane> <words...>` is run without `--chunk`
- **THEN** one `pane.send_text` request carries the words joined by single spaces

### Requirement: Pieces never split a UTF-8 character

Every piece SHALL be valid UTF-8 and the pieces SHALL concatenate to the
original text. A piece SHALL exceed BYTES only when it is a single character
wider than BYTES.

#### Scenario: Multibyte text splits on character boundaries

- **WHEN** text mixing emoji, CJK and accented Latin is chunked at any limit
- **THEN** each piece is valid UTF-8 and the pieces rejoin to the original text
