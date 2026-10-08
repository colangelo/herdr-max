## 1. CLI

- [x] 1.1 Parse `--chunk` / `--chunk-delay` (space and `=` forms, anywhere after the pane id, `--` terminator) in `pane send-text`
- [x] 1.2 Split on UTF-8 boundaries and send one `pane.send_text` per piece with the delay between, returning after the last
- [x] 1.3 Help text: explain paste collapse and point at `--chunk`; keep the `pane run` next-step hint

## 2. Tests

- [x] 2.1 Parser: unchanged unchunked join, option placement, default delay, `--`, errors
- [x] 2.2 Splitter: byte limit, emoji/CJK/ZWJ never cut, empty and short text
- [x] 2.3 Help test covers the new options and the paste hint

## 3. Measurement and docs

- [x] 3.1 Raw-reader read-size sweep and Claude Code end-to-end sweep in a throwaway session; record in design.md
- [x] 3.2 Document the flag in `docs/next/website/src/content/docs/cli-reference.mdx`

## 4. Verification

- [x] 4.1 `cargo nextest run --locked` (3936 passed, twice) + `cargo fmt --check` green; `just check` and clippy are blocked on m4m until https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/80
