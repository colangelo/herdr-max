# UI style guide

One rule sheet so herdr's overlays, its mode bars and the popup programs that run inside herdr
(the asks pane) look like one program. Follow it for any new screen. Origin:
`openspec/changes/unify-tui-look` and
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/174.

## Palette

Catppuccin tokens, with the nearest xterm-256 colour for curses programs (curses has no true
colour; numbers picked by smallest RGB distance).

| Role | Name | Hex | 256 | Use |
|---|---|---|---|---|
| accent | `accent` | `#89b4fa` | 111 | frame, selection bar, keys, primary button, section headings |
| overlay background | `panel_bg` | `#181825` | 234 | overlay background; dark text on the accent bar |
| secondary fill | `surface0` | `#313244` | 237 | secondary buttons, settings-style selection, text field |
| rule | `surface1` | `#45475a` | 239 | rules (`─`) |
| dim | `overlay0` | `#6c7086` | 243 | labels, subtitles, counts, metadata, done rows |
| text | `text` | `#cdd6f4` | 189 | normal and bold text |
| name | `mauve` | `#cba6f7` | 183 | ids and names next to a heading (keybinds, todo owners) |
| good | `green` | `#a6e3a1` | 151 | done, saved, idle |
| attention | `yellow` | `#f9e2af` | 223 | working, changed, needs a look |
| bad | `red` | `#f38ba8` | 211 | urgent, failed, offline |
| choice | (none) | `#afafff` | 147 | asks pane option labels and descriptions only |

`accent` (111) and choice (147) are both light blue and must never mean the same thing: 111 is a
background (the bar) or a key; 147 is the text of an option that is not under the cursor.

## Frame and title

- Herdr draws the frame: single line, `accent`, on `panel_bg`. A program inside a herdr popup draws
  no frame of its own. Never draw a second frame inside the first.
- Row 1: the title in bold `text`, lowercase (`keybinds`, `settings`, `todos/notes`, `asks`),
  inset one column. A count or position sits right-aligned in dim on the same row (`25 panes`,
  `74 todos`, `30 open`).
- Row 2, when there is one: a dim subtitle (a hint, a summary, or the active filter). A filter that
  is on is shown in `accent`; a filter that is off is dim.
- Then a rule, or a blank row, before the body.

## Rules and spacing

- A rule is `─` in `surface1`, inset one column on both sides.
- A short label may sit at the start of a rule (`infra · 4 panes────`, as the navigator does). A
  connection banner goes there.
- Content is inset one column. Separate groups with one blank row.
- One row separates the body from the footer. A status message, when there is one, uses that row.

## Selection

- Primary lists: the whole row is the `accent` bar with `panel_bg` text in bold. No pointer glyph
  (`❯`) and no reverse video. Marks on the row take the bar's text colour. In curses: background
  111, foreground 234, bold; without colours, fall back to reverse video.
- Settings-style lists (pick one value): `surface0` bar with bold `text`, `▸` in front, `✓` after
  the current value.
- Tabs and chips: the active one is an `accent` chip. The others are dim text, with their mark in
  its state colour.

## State colours and marks

One meaning per colour:

- green = done or saved
- yellow = changed or working
- red = urgent or failed
- `accent` = the selected thing or a key
- dim = secondary

Each glyph keeps its colour wherever it appears. Asks pane marks: `☐` open is `text`, `✔` answered
is green, `!` changed is yellow, `·` closed is dim. Keep the asks glyphs as they are; the asks web
page and Claude Code's question tool use the same ones. Done items may be struck through and dim
(todos). Priority bullets (`▲ ● ▼`) keep their colours: shape and colour both carry the priority.
A scroll hint (`more lines`, `↓ 5 more`) is dim, not yellow.

## Mode colour

A mode that has its own colour keeps it. Its chip and its keys use that colour so modes tell apart
at a glance. The layout is the same for every mode: one filled chip with the mode name in dark bold
text, keys bold in the mode's colour, labels dim, secondary information as dim labels, and no
second chip. `accent` is the colour for modes without one of their own and for everything that is
not a mode bar. A new mode picks a palette colour no other mode uses and keeps it. Mode colours are
a separate set from state colours: on a mode bar, red means "PANES mode", not "failed".

| Mode | Colour | Hex | 256 |
|---|---|---|---|
| PREFIX, SCROLL, COPY, NAVIGATE | `accent` | `#89b4fa` | 111 |
| PANES (display panes) | soft red (`red`) | `#f38ba8` | 211 |
| RESIZE | `mauve` | `#cba6f7` | 183 |
| SYNC (`SYNC N panes`) | `SYNC_YELLOW` | `#ffd60a` | 220 |

SYNC's yellow is a fixed colour (`SYNC_YELLOW`), not the palette's soft `yellow` (`#f9e2af`). Keep it yellow.

Which file draws which bar moves with the code: the current map is in
`openspec/changes/unify-tui-look/tasks.md` (phase 3 file map).

## Footer

- Browsing and editing views end with one key bar: `key label` pairs, the key in bold `accent`
  (or the mode's colour on a mode bar), the label in dim, two spaces between pairs, inset one
  column. Pairs that do not fit are dropped from the right.
- A key that does nothing right now is drawn all dim, or left out.
- Modals that end a task use a button row instead: centred chips, `↵ label` primary in `accent`
  with dark bold text, the rest in `surface0` with bold `text`, two spaces apart.
- Use the key bar for views you browse or edit in; use the button row for a modal that ends a task
  (apply, confirm, open). A view never has both. The primary chip is the one action `↵` performs
  (`↵ apply` in settings, `↵ open pane` in todos/notes).

## Text, numbers, truncation

- Cut long text with `…`; never wrap it into the next column.
- Metadata (owner, key, age, turn, `#N`) is dim and sits on the right. Right-align numbers.
- When a row is too narrow, drop metadata first, then cut the title. The title column is the last
  thing to go. Asks list order of loss: turn, then owner, then key, then cut the title.
- A list has no column-header row: columns run mark, header, title on the left and own, key, age,
  turn dim on the right.

## Empty, loading and error states

- Empty: one dim line in the body, inset one column: `no open asks`, `loading…`,
  `no ask matches the filter`.
- Too small: one yellow line, `widen the popup — asks needs 60×12, this is W×H`.
- A failed action: the status row in red; a success: the status row in green; a note: dim.
- Lost connection: a red label at the start of the rule under the title
  (`▲ bus unreachable — read-only, retrying`), a yellow status row, and the keys that would write
  drawn dim.
- A popup program keeps the terminal's own background; herdr's popup frame marks the edge. Do not
  paint `panel_bg` in a 256-colour program, because the nearest grey (234) is neutral, not
  catppuccin's blue-black.
