# Design: paced `pane send-text`

## Where the pacing lives: the CLI

`pane send-text --chunk N` loops over the existing `pane.send_text` request, one
request per piece, sleeping between them. That is the path the reporter already
proved (separate `send-text` calls arrive whole), it blocks until the last piece
is written for free, and it needs no API field, protocol bump or server change,
so it works against any running server.

The server-side alternative, optional chunk parameters on `pane.send_text`,
cannot sleep in the app loop. It would need async paced writes plus a
completion signal (`send_bytes_after` in `src/pane.rs` has no ack; upstream's
equivalent for `agent prompt` is ~980 lines in 8633a398). Raw socket API
clients do not get `--chunk`; they can pace their own `pane.send_text` calls the
same way.

## Splitting

Pieces are at most N **bytes**, cut only on UTF-8 character boundaries. A
single character wider than N becomes a piece of its own, so every piece is
non-empty valid UTF-8 and the concatenation is the original text. Grapheme
clusters (a ZWJ emoji) may be split between scalars; the receiver reassembles
them because the bytes arrive in order and each piece is valid UTF-8.

## Parsing

The hand-rolled parser recognises `--chunk`/`--chunk-delay` (space or `=`
form) anywhere after the pane id, as clap's help implies; `--` ends option
parsing. Every other word is text joined with single spaces, exactly as before.
The one behaviour change without `--chunk`: a text word that is literally
`--chunk`, `--chunk-delay` or `--` is now an option; `--` escapes it.
`--chunk-delay` without `--chunk` is an error rather than silently ignored.

## Default delay: 20 ms, measured

Measured on m4m, `0.8.2-ac-beta.91` server, headless throwaway session, 3000
bytes as ten 300-byte pieces.

Raw reader (Python, raw-mode tty, logs each `read()` size and time): every
delay, **including 0 ms**, arrived as ten separate 300-byte reads (0 ms: reads
0.2 ms apart; the per-request round trip alone separates them). One unchunked
3000-byte send arrived as three reads (1022/1022/956) within 0.2 ms. So herdr
and the PTY do not coalesce; the receiver's own read loop decides.

Claude Code 2.1.281 (idle, input box; placeholder read from the screen, bytes
checked by a Ctrl+G `$EDITOR` that copies the box to a file — nothing was
submitted), three runs each:

| send | placeholder | bytes exact |
|---|---|---|
| unchunked 3000 | yes | yes |
| `--chunk 300 --chunk-delay 0` | yes (3/3) | yes |
| `--chunk-delay 1` (≈1.7 ms actual gap) | no (3/3) | yes |
| `--chunk-delay 2`, `5`, `10`, `20` | no (3/3 each) | yes |
| `--chunk 600 --chunk-delay 20` | no | yes |
| `--chunk 900 --chunk-delay 20` | yes | yes |
| 4050-byte emoji/CJK/accented text, `--chunk 300` | no | yes |

The smallest gap that works is ~1 ms; zero does not. 20 ms is more than ten
times that, leaves room for a busy receiver whose event loop reads late, and
sits near the ~12 ms the reporter's separate CLI calls had between them.
3000 bytes at 300 per piece costs ~0.2 s. The docs recommend `--chunk 300`:
900-byte pieces still collapse even with a gap.
