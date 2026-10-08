# Send large text to a pane in paced pieces

## Why

A single `herdr pane send-text` of roughly 1000 characters or more into a Claude
Code pane arrives collapsed as `[Pasted text #1][Pasted text #2]…`. After a typed
`/compact `, only the last ~190 characters survive, and `/compact` never expands
the placeholders. Four sessions compacted on truncated instructions before this
was traced (https://gitea.cat-bluegill.ts.net/Aruba/mig-svn-bm/issues/51#issuecomment-13042).

`pane send-text` does not use bracketed paste: `handle_pane_send_text` does one
raw PTY write. Claude Code classifies an input burst as a paste by its size
alone. Measured on beta.91: 600 and 800 characters in one send stay plain,
1000+ collapse, and 3000 characters sent as ten separate 300-character CLI
calls arrive whole. A "no bracketed paste" flag would therefore change nothing.

## What Changes

`herdr pane send-text` gains two opt-in options:

- `--chunk <BYTES>` splits the text into pieces of at most that many bytes, on
  UTF-8 character boundaries, and sends each piece as its own `pane.send_text`
  request.
- `--chunk-delay <MS>` sets the pause between pieces (default measured, see
  design). The pause is what keeps the pieces apart: back-to-back writes can
  coalesce into one read on the receiving side, which is still one burst.

The command returns only after the last piece has been sent, so a caller can
read the input box back or press Enter immediately after.

Without `--chunk`, behaviour is byte-for-byte unchanged.

## Impact

- Affected code: `src/cli/pane.rs` (`pane send-text` parsing and sending),
  `src/cli/spec.rs` (help), `docs/next/.../cli-reference.mdx`
- No server, API, protocol or config change. The pacing lives in the CLI, so it
  works against any server version; raw socket API clients do not get it and
  can chunk themselves the same way.

## Non-goals

- A `--typed` or "no bracketed paste" mode: `send-text` already is not
  bracketed, so it would do nothing.
- Changing `agent prompt`, which is bracketed on purpose.
- Server-side paced writes with a completion signal.
