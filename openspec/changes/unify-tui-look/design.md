## Context

https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/174. ac wants herdr's screens to look
like one program, and the asks pane most of all. The asks pane is a separate Python curses program
(`CONTEXT/SKILLS/gestore/asks_tui.py`, owned by the `context` session) that runs inside a herdr
popup, so it can be fixed now without touching herdr.

Sources read for this design:

- nine screenshots, `~/Pictures/Screenshots/herdr-tui - 2026-10-06` (asks pane, keybinds,
  settings, notifications, todos panel, status bars, navigator, todos/notes);
- the fork's drawing code: `src/ui/widgets.rs` (panel shell, title inset), `src/app/state.rs`
  (`Palette::catppuccin`), `src/ui/display_panes.rs` (the PANES bar);
- the look that ships with the 0.9.3 cutover: upstream `src/client/shell/overlays.rs` (panel,
  buttons, help, navigator, menus) and `src/client/shell/render.rs` (the mode bar);
- `asks_tui.py` (`build_attrs`, `footer`, `render_list`, `render_answer`) and
  `asks_page/ANSWER-UI.md`.

Upstream has no design document. A search of herdrdev/herdr issues found only bugs that touch
the look: #2880 (the settings hint bar shows `↑↓ select` where it does nothing), #4415 (stray line
fragments in the navigator), and theme bugs (#1307, #3262). None of them sets a direction.

What the code shows: herdr already has one panel (accent single-line frame on `panel_bg`, bold
title, one-column inset) and one selection (an accent bar with dark bold text). The footer is not
one thing, even upstream. 0.9.3 has three kinds: the mode bar puts keys first in bold accent with
labels in `overlay0` (`esc back  ↑/↓ workspace`). The help and navigator overlays print one flat
`overlay0` string with labels first (`search / · scroll j/k`). Modals use button chips. The rules
below pick one key bar for browsing views and keep button chips for modals.

## Goals / Non-Goals

**Goals:**

- One short rule sheet that herdr overlays and popup programs follow.
- The asks pane restyled to it (phase 2), with mockups ac approves first.
- The two herdr screens that clash most, mocked, to fix after the 0.9.3 cutover (phase 3).

**Non-Goals:**

- Any herdr code change in this phase.
- New keys or new behaviour in the asks pane. Only colour, order and spacing change.
- Light themes in the asks pane. It uses fixed 256-colour numbers picked for catppuccin, ac's
  theme (see Risks).

## Style rules

### Palette

The catppuccin tokens herdr already uses, with the nearest xterm-256 colour for curses programs
(curses has no true colour). The numbers were picked by smallest RGB distance.

| Token | Hex | 256 | Used for |
|---|---|---|---|
| `accent` | `#89b4fa` | 111 | frame, selection bar, keys, primary button, section headings |
| `panel_bg` | `#181825` | 234 | overlay background; dark text on the accent bar |
| `surface0` | `#313244` | 237 | secondary buttons, settings-style selection, text field |
| `surface1` | `#45475a` | 239 | rules (`─`) |
| `overlay0` | `#6c7086` | 243 | dim: labels, subtitles, counts, metadata, done rows |
| `text` | `#cdd6f4` | 189 | normal and bold text |
| `mauve` | `#cba6f7` | 183 | ids and names next to a heading (keybinds, todo owners) |
| `green` | `#a6e3a1` | 151 | done, saved, idle |
| `yellow` | `#f9e2af` | 223 | working, changed, needs a look |
| `red` | `#f38ba8` | 211 | urgent, failed, offline |
| choice | `#afafff` | 147 | asks pane option labels and descriptions only (ac's pick, kept) |

`accent` (111) and choice (147) are both light blue. They never meet as the same thing: 111 is a
**background** (the bar) or a key, 147 is the **text** of an option that is not under the cursor.

### Frame and title

- Herdr draws the frame: single line, `accent`, on `panel_bg`. A program inside a herdr popup
  draws **no frame of its own**. Nothing draws a second frame inside the first.
- Row 1: the title in bold `text`, **lowercase** like herdr's (`keybinds`, `settings`,
  `todos/notes`, so `asks`), inset one column. A count or position sits right-aligned in dim on
  the same row (`25 panes`, `74 todos`, `30 open`).
- Row 2, when there is one: a dim subtitle (a hint, a summary, or the active filter). A filter
  that is on is shown in `accent`, a filter that is off in dim.
- Then a rule, or a blank row, before the body.

### Rules and spacing

- A rule is `─` in `surface1`, inset one column on both sides.
- A short label may sit at the start of a rule (`infra · 4 panes────`, as the navigator does).
  That is where a connection banner goes.
- Content is inset one column. Groups are separated by one blank row.
- One row separates the body from the footer. A status message, when there is one, uses that row.

### Selection

- Primary lists: the whole row is the `accent` bar with `panel_bg` text in bold. No pointer glyph
  (`❯`) and no reverse video. Marks on the row take the bar's text colour.
- Settings-style lists (pick one value): `surface0` bar with bold `text`, `▸` in front, `✓` after
  the current value. This is herdr's settings panel and is unchanged.
- Tabs and chips: the active one is an `accent` chip. The others are dim text, with their mark
  in its state colour.

### State colours and marks

One meaning per colour: green = done or saved, yellow = changed or working, red = urgent or failed,
`accent` = the selected thing or a key, dim = secondary. Each glyph keeps its colour wherever it
appears. In the asks pane, `☐` open is `text`, `✔` answered is green, `!` changed is yellow, and
`·` closed is dim. The asks glyphs stay as they are, because the asks web page and Claude Code's
question tool use the same ones. Done items may be struck through and dim (todos).

### Footer

- **Browsing and editing views end with one key bar:** `key label` pairs, the key in bold
  `accent`, the label in dim, two spaces between pairs, inset one column. This is the mode bar ac
  sees all day, in the fork and in 0.9.3. Pairs that do not fit are dropped from the right.
- **A key that does nothing right now is drawn all dim, or left out** (the upstream #2880 rule).
- **Modals that end a task use a button row instead:** centred chips, `↵ label` primary in
  `accent` with dark bold text, the rest in `surface0` with bold `text`, two spaces apart. A view
  never has both a key bar and a button row.
- **A mode bar uses the mode's colour** (see Mode colour below).

### Mode colour

A mode that has its own colour **keeps it**. Its chip and its keys use that colour, so you can
tell a mode apart at a glance. The layout is the same for every mode: one filled chip with the
mode name in dark bold text, keys bold in the mode's colour, labels dim, secondary information as
dim labels, and **no second chip**. `accent` is the colour for modes without one of their own,
and for everything that is not a mode bar. A future mode (green, orange…) picks a palette colour
that no other mode uses and keeps it. ac set this rule on 2026-10-08: *"for an alt-modality like
the one in D i prefer it in red"*, and *"there is also the sync mode that is yellow, right? there
could be a green one maybe, an orange, it depends, but in that case the color should be kept"*.

The modes today, as the code draws them:

| Mode | Colour | Hex | 256 | Where |
|---|---|---|---|---|
| PREFIX, SCROLL, COPY, NAVIGATE | `accent` | `#89b4fa` | 111 | `src/ui/menus.rs` (`mode_style`) |
| PANES (display panes) | `red` | `#f38ba8` | 211 | `src/ui/display_panes.rs` (`mode_chip_style`) |
| RESIZE | `mauve` | `#cba6f7` | 183 | `src/ui/display_panes.rs`; upstream `client/shell/render.rs` |
| SYNC (`SYNC N panes`) | `SYNC_YELLOW` | `#ffd60a` | 220 | `src/ui.rs` (`SYNC_YELLOW` in `src/app/state.rs`) |

SYNC's yellow is a fixed colour, not the palette's soft `yellow` (`#f9e2af`). It stays as it is,
because a mode keeps its colour. Mode colours are a separate set from the state colours: on a
mode bar, red means "PANES mode", not "failed".

### Text, numbers, truncation

- Long text is cut with `…`, never wrapped into the next column.
- Metadata (owner, key, age, turn, `#N`) is dim and sits on the right. Numbers are right-aligned.
- When a row is too narrow, metadata goes first, then the title is cut. The title column is the
  last thing to go.

### Empty, loading and error states

- Empty: one dim line in the body, inset one column: `no open asks`, `loading…`,
  `no ask matches the filter`.
- Too small: one yellow line, `widen the popup — asks needs 60×12, this is W×H` (unchanged).
- A failed action: the status row in red; a success: the status row in green; a note: dim.
- Lost connection: a red label at the start of the rule under the title
  (`▲ bus unreachable — read-only, retrying`), a yellow status row, and the keys that would write
  are drawn dim.

## Decisions

1. **Key bar = keys first, bold accent keys, dim labels.** Asks already puts keys first and ac
   asked to keep "herdr-style key bars". The mode bar is the herdr key bar people see most. The
   change for asks: keys turn from white to blue, labels from white to grey. *Instead:* the flat
   label-first strings of 0.9.3's help and navigator footers. They are the odd ones out, and a
   phase 3 item could bring them in line.
2. **Selection = accent bar, no `❯`.** Navigator, notifications, todos, and 0.9.3's navigator and
   menus all do this. In curses: background 111, foreground 234, bold. Without colours, it falls
   back to reverse video, as today.
3. **147 stays for option labels and descriptions.** ac chose it. Only the option under the
   cursor changes: it becomes the accent bar, not reverse video.
4. **Lowercase `asks` title, counts on the right.** This is the pattern of every herdr overlay
   title.
5. **The list loses its column-header row.** Its columns become mark · header · title on the
   left, with own · key · age · turn dim on the right, like a notifications row. The values
   explain themselves (`lab`, `i12`, `4d`, `T1`), and the title gets the room. When narrow:
   drop turn, then owner, then key, then cut the title.
6. **The bus banner sits in the rule row**, in red, as a rule label. Read-only mode dims `enter`.
7. **`more lines` hint is dim, not yellow.** Yellow means "needs a look". A scroll hint does not,
   and herdr's sidebar draws `↓ 5 more` dim.
8. **The asks pane keeps the terminal's own background.** The nearest 256 grey to `panel_bg` (234)
   is neutral grey, not catppuccin's blue-black, so painting it would look like a patch. Herdr's
   popup frame already marks the edge.
9. **Herdr's PANES bar uses one chip, in its own red.** Today it has a red `PANES` chip, red keys,
   and a second red `VERSION` chip. After: one red `PANES` chip with dark bold text, bold red keys,
   and `version` as a dim label with no chip. The red is the current soft red, `palette.red`
   `#f38ba8` (256: 211). ac, 2026-10-08: *"for an alt-modality like the one in D i prefer it in red.
   keep your design but in that modality use the current red (that's not intese) where it's
   blue"*. So the clash this fixes is the second chip, not the colour: each mode keeps its own
   colour (see Mode colour).
10. **The todos/notes panel loses its inner frame.** The count moves to the title row, search
    moves to the subtitle row, and the preview moves under the list, after a rule. `↵ open pane`
    becomes the accent primary button, the same as `↵ apply` in settings. The bullet glyphs
    (`▲ ● ▼`) keep their colours: shape and colour both carry the priority.

## Mockups

All mockups are 80 cells wide inside the herdr popup frame (82 with it), redrawn from the code at
that size; the ask data is made up. Each mockup has two blocks of the same size. The first is the
text. The second is its **paint map**: each cell holds a code for that cell's style. A blank in
the paint map means a plain space.

Paint codes, AFTER: `B` bold text · `.` text · `d` dim (`overlay0`) · `k` key (bold `accent`) ·
`A` selection bar or primary chip (`accent` background, dark bold text) · `U` secondary chip
(`surface0` background) · `S` text field (`surface0` background) · `c` choice 147 · `g` green ·
`y` yellow · `r` red · `m` mauve · `x` struck-through dim · `s` rule (`surface1`) · `f` herdr's
frame · `P` mode chip in red (`red` background, dark bold text) · `p` key in red (bold `red`).

Paint codes, BEFORE: `b` terminal bold (white) · `.` terminal default · `D` curses dim · `R`
reverse video · `C` choice 147 · `G` green · `Y` yellow bold · `E` red bold · `P` red chip ·
`p` red bold key.

### What changes from what ac already chose

- Option labels stay **147 light blue**. The option under the cursor becomes a **blue bar** (was
  white reverse video).
- The key bar stays **key then label**. Keys become **bold blue** (were bold white). Labels become
  **grey** (were white).
- The selected ask in the list is a **blue bar** (was white reverse video on the left columns
  only, with `❯`).
- Keys do not change: j/k move, h/l previous/next question, q/Esc back to the list, h in the
  list = answered only, o = webpage.

### A. Asks list

BEFORE (today; the reverse video covers only the left columns, as in screenshot 1):

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ Asks — open 30 (lab 30 · aruba 0)                 filter: owner all · age all  │
│────────────────────────────────────────────────────────────────────────────────│
│   own   key      age  since header         title                               │
│ ❯ lab   i12      4d   T1    ☐ gh tokens     Revoke the two leaked GitHub CLI … │
│   lab   i16      4d   T1    ☐ Firefox test  Test the Firefox tab-close safety… │
│   lab   i17      4d   T1    ✔ Photo + Find  Two checks in the Direction app o… │
│   lab   vik1     4d   T295  ☐ Run the Cal…  Run the CalDAV spike (ac/infra#11… │
│   lab   i34      4d   T322  ! disk banner   Click Allow on one test banner at… │
│   lab   a1004-26 1d   T698  ☐ CPU levers    m4m has no idle CPU during builds… │
│   aruba a1005-01 1d   T803  ☐ uzi role      What role should uzi play in herd… │
│   lab   a1005-16 1d   T860  ✔ gmail Max     Login A (the gmail Claude Max sub… │
│   lab   a1005-18 15h  T883  ☐ CPA extras    After the v8 upgrade: do we add C… │
│                                                                                │
│                                                                                │
│                                                                                │
│────────────────────────────────────────────────────────────────────────────────│
│                                                                                │
│ enter answer  / text  a owner  g age  h answered  o webpage  q close           │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f bbbb b bbbb bb bbbb bb b bbbbb bb                 bbbbbbb bbbbb bbb b bbb bbb  f
fDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDf
f   DDD   DDD      DDD  DDDDD DDDDDD         DDDDD                               f
fRRRRRRRRRRRRRRRRRRRRRRRRRRRRR. .. ......     ...... ... ... ...... ...... ... . f
f   ...   ...      ..   ..    . ....... ....  .... ... ....... ......... ....... f
f   ...   ...      ..   ..    G ..... . ....  ... ...... .. ... ......... ... .. f
f   ...   ....     ..   ....  . ... ... ....  ... ... ...... ..... ............. f
f   ...   ...      ..   ....  Y .... ......   ..... ..... .. ... .... ...... ... f
f   ...   ........ ..   ....  . ... ......    ... ... .. .... ... ...... ....... f
f   ..... ........ ..   ....  . ... ....      .... .... ...... ... .... .. ..... f
f   ...   ........ ..   ....  G ..... ...     ..... . .... ..... ...... ... .... f
f   ...   ........ ...  ....  . ... ......    ..... ... .. ........ .. .. ... .. f
f                                                                                f
f                                                                                f
f                                                                                f
fDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDf
f                                                                                f
f bbbbb ......  b ....  b .....  b ...  b ........  b .......  b .....           f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


AFTER:

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ asks                                                                   30 open │
│ lab 30 · aruba 0 · owner all · age all                                         │
│ ────────────────────────────────────────────────────────────────────────────── │
│ ☐ gh tokens     Revoke the two leaked GitHub CLI tok…  lab   i12       4d   T1 │
│ ☐ Firefox test  Test the Firefox tab-close safety ch…  lab   i16       4d   T1 │
│ ✔ Photo + Find  Two checks in the Direction app on t…  lab   i17       4d   T1 │
│ ☐ Run the Cal…  Run the CalDAV spike (ac/infra#113):…  lab   vik1      4d T295 │
│ ! disk banner   Click Allow on one test banner at m4…  lab   i34       4d T322 │
│ ☐ CPU levers    m4m has no idle CPU during builds: w…  lab   a1004-26  1d T698 │
│ ☐ uzi role      What role should uzi play in herdr's…  aruba a1005-01  1d T803 │
│ ✔ gmail Max     Login A (the gmail Claude Max subscr…  lab   a1005-16  1d T860 │
│ ☐ CPA extras    After the v8 upgrade: do we add Code…  lab   a1005-18 15h T883 │
│                                                                                │
│                                                                                │
│                                                                                │
│                                                                                │
│                                                                                │
│ enter answer  / text  a owner  g age  h answered  o webpage  q close           │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB                                                                   dd dddd f
f ddd dd d ddddd d d ddddd ddd d ddd ddd                                         f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f . BBBBBBB BBBB  .... ... ....... ......... ...... ...  ddd   ddd       dd   dd f
f g ddddd d dddd  ddd dddddd dd ddd ddddddddd ddd dd dd  ddd   ddd       dd   dd f
f . BBB BBB BBBB  ... ... ...... ..... ................  ddd   dddd      dd dddd f
f y BBBB BBBBBB   ..... ..... .. ... .... ...... .. ...  ddd   ddd       dd dddd f
f . BBB BBBBBB    ... ... .. .... ... ...... ....... ..  ddd   dddddddd  dd dddd f
f . BBB BBBB      .... .... ...... ... .... .. ........  ddddd dddddddd  dd dddd f
f g ddddd ddd     ddddd d dddd ddddd dddddd ddd ddddddd  ddd   dddddddd  dd dddd f
f . BBB BBBBBB    ..... ... .. ........ .. .. ... .....  ddd   dddddddd ddd dddd f
f                                                                                f
f                                                                                f
f                                                                                f
f                                                                                f
f                                                                                f
f kkkkk dddddd  k dddd  k ddddd  k ddd  k dddddddd  k ddddddd  k ddddd           f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


### B. Asks answering view

BEFORE:

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ Asks  ‹ +1  ☐ Firefox test  ✔ Photo + Find  ☐ CalDAV  ! disk banner  +25 ›     │
│────────────────────────────────────────────────────────────────────────────────│
│ Test the Firefox tab-close safety check on m4m, together with direction-lead   │
│ (about 5 minutes at the keyboard).                                             │
│                                                                                │
│ why    direction-lead added a guard so closing the last Firefox tab asks       │
│        first; it needs a real close on m4m to prove it.                        │
│ waits  direction-lead (pane w3:p2), record ac/direction#412                    │
│ asked  i16 · 4d · since T1 · by gestore-lab                                    │
│                                                                                │
│                                                                                │
│────────────────────────────────────────────────────────────────────────────────│
│ ❯ 1. Do it now (Recommended)                                                   │
│        I sit at m4m with direction-lead for five minutes                       │
│   2. Tomorrow morning                                                          │
│        direction-lead keeps the guard off until then                           │
│   3. Skip the test ✔                                                           │
│        ship the guard untested                                                 │
│   4. Type something.                                                           │
│ ────────────────────────────────────────────────────────────────────────────── │
│   5. Later                                                                     │
│ rec: do it now — the guard ships Sunday with 0.9.3 and nothing else tests it.  │
│                                                                                │
│ enter save  ↑↓ 1-9 move  PgDn/e explanation  ←→ h/l ask  q list  o webpage     │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f bbbb  D DD RRRRRRRRRRRRRRRR G GGGGG G GGGG  . ......  Y YYYY YYYYYY  DDD D     f
fDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDf
f bbbb bbb bbbbbbb bbbbbbbbb bbbbbb bbbbb bb bbbb bbbbbbbb bbbb bbbbbbbbbbbbbb   f
f bbbbbb b bbbbbbb bb bbb bbbbbbbbbb                                             f
f                                                                                f
f DDD    .............. ..... . ..... .. ....... ... .... ....... ... ....       f
f        ...... .. ..... . .... ..... .. ... .. ..... ...                        f
f DDDDD  .............. ..... ....... ...... ................                    f
f DDDDD  ... . .. . ..... .. . .. ...........                                    f
f                                                                                f
f                                                                                f
fDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDf
fRRRRRRRRRRRRRRRRRRRRRRRRRRRRR                                                   f
f        C CCC CC CCC CCCC CCCCCCCCCCCCCC CCC CCCC CCCCCCC                       f
f   CC CCCCCCCC CCCCCCC                                                          f
f        CCCCCCCCCCCCCC CCCCC CCC CCCCC CCC CCCCC CCCC                           f
f   CC CCCC CCC CCCC G                                                           f
f        CCCC CCC CCCCC CCCCCCCC                                                 f
f   CC CCCC CCCCCCCCCC                                                           f
f DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD f
f   CC CCCCC                                                                     f
f DDDD DD DD DDD D DDD DDDDD DDDDD DDDDDD DDDD DDDDD DDD DDDDDDD DDDD DDDDD DDD  f
f                                                                                f
f bbbbb ....  bb bbb ....  bbbbbb ...........  bb bbb ...  b ....  b .......     f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


AFTER:

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ asks                                                2 of 30 · lab 30 · aruba 0 │
│ ‹ +1  ☐ Firefox test  ✔ Photo + Find  ☐ CalDAV  ! disk banner  +25 ›           │
│ ────────────────────────────────────────────────────────────────────────────── │
│ Test the Firefox tab-close safety check on m4m, together with direction-lead   │
│ (about 5 minutes at the keyboard).                                             │
│                                                                                │
│ why    direction-lead added a guard so closing the last Firefox tab asks       │
│        first; it needs a real close on m4m to prove it.                        │
│ waits  direction-lead (pane w3:p2), record ac/direction#412                    │
│ asked  i16 · 4d · since T1 · by gestore-lab                                    │
│                                                                                │
│ ────────────────────────────────────────────────────────────────────────────── │
│ 1. Do it now (Recommended)                                                     │
│    I sit at m4m with direction-lead for five minutes                           │
│ 2. Tomorrow morning                                                            │
│    direction-lead keeps the guard off until then                               │
│ 3. Skip the test ✔                                                             │
│    ship the guard untested                                                     │
│ 4. Type something.                                                             │
│ ────────────────────────────────────────────────────────────────────────────── │
│ 5. Later                                                                       │
│ rec: do it now — the guard ships Sunday with 0.9.3 and nothing else tests it.  │
│                                                                                │
│ enter save  ↑↓ 1-9 move  PgDn/e explanation  ←→ h/l ask  q list  o webpage     │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB                                                d dd dd d ddd dd d ddddd d f
f d dd AAAAAAAAAAAAAAAA g ddddd d dddd  . dddddd  y dddd dddddd  ddd d           f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f BBBB BBB BBBBBBB BBBBBBBBB BBBBBB BBBBB BB BBBB BBBBBBBB BBBB BBBBBBBBBBBBBB   f
f BBBBBB B BBBBBBB BB BBB BBBBBBBBBB                                             f
f                                                                                f
f ddd    .............. ..... . ..... .. ....... ... .... ....... ... ....       f
f        ...... .. ..... . .... ..... .. ... .. ..... ...                        f
f ddddd  .............. ..... ....... ...... ................                    f
f ddddd  ... . .. . ..... .. . .. ...........                                    f
f                                                                                f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f    c ccc cc ccc cccc cccccccccccccc ccc cccc ccccccc                           f
f cc cccccccc ccccccc                                                            f
f    cccccccccccccc ccccc ccc ccccc ccc ccccc cccc                               f
f cc cccc ccc cccc g                                                             f
f    cccc ccc ccccc cccccccc                                                     f
f cc cccc cccccccccc                                                             f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f cc ccccc                                                                       f
f dddd dd dd ddd d ddd ddddd ddddd dddddd dddd ddddd ddd ddddddd dddd ddddd ddd  f
f                                                                                f
f kkkkk dddd  kk kkk dddd  kkkkkk ddddddddddd  kk kkk ddd  k dddd  k ddddddd     f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


AFTER, while typing an answer in `Type something.` (the field is a `surface0` strip, as in herdr's
rename box):

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ asks                                                2 of 30 · lab 30 · aruba 0 │
│ ‹ +1  ☐ Firefox test  ✔ Photo + Find  ☐ CalDAV  ! disk banner  +25 ›           │
│ ────────────────────────────────────────────────────────────────────────────── │
│ Test the Firefox tab-close safety check on m4m, together with direction-lead   │
│ (about 5 minutes at the keyboard).                                             │
│                                                                                │
│ why    direction-lead added a guard so closing the last Firefox tab asks       │
│        first; it needs a real close on m4m to prove it.                        │
│ waits  direction-lead (pane w3:p2), record ac/direction#412                    │
│ asked  i16 · 4d · since T1 · by gestore-lab                                    │
│ ────────────────────────────────────────────────────────────────────────────── │
│ 1. Do it now (Recommended)                                                     │
│    I sit at m4m with direction-lead for five minutes                           │
│ 2. Tomorrow morning                                                            │
│    direction-lead keeps the guard off until then                               │
│ 3. Skip the test ✔                                                             │
│    ship the guard untested                                                     │
│ 4. Type something.                                                             │
│     > after lunch, 14:00▏                                                      │
│ ────────────────────────────────────────────────────────────────────────────── │
│ 5. Later                                                                       │
│ rec: do it now — the guard ships Sunday with 0.9.3 and nothing else tests it.  │
│                                                                                │
│ enter save  alt-enter new line  esc keep draft  ctrl-u clear                   │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB                                                d dd dd d ddd dd d ddddd d f
f d dd AAAAAAAAAAAAAAAA g ddddd d dddd  . dddddd  y dddd dddddd  ddd d           f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f BBBB BBB BBBBBBB BBBBBBBBB BBBBBB BBBBB BB BBBB BBBBBBBB BBBB BBBBBBBBBBBBBB   f
f BBBBBB B BBBBBBB BB BBB BBBBBBBBBB                                             f
f                                                                                f
f ddd    .............. ..... . ..... .. ....... ... .... ....... ... ....       f
f        ...... .. ..... . .... ..... .. ... .. ..... ...                        f
f ddddd  .............. ..... ....... ...... ................                    f
f ddddd  ... . .. . ..... .. . .. ...........                                    f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f cc cc cc ccc ccccccccccccc                                                     f
f    c ccc cc ccc cccc cccccccccccccc ccc cccc ccccccc                           f
f cc cccccccc ccccccc                                                            f
f    cccccccccccccc ccccc ccc ccccc ccc ccccc cccc                               f
f cc cccc ccc cccc g                                                             f
f    cccc ccc ccccc cccccccc                                                     f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f    SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f cc ccccc                                                                       f
f dddd dd dd ddd d ddd ddddd ddddd dddddd dddd ddddd ddd ddddddd dddd ddddd ddd  f
f                                                                                f
f kkkkk dddd  kkkkkkkkk ddd dddd  kkk dddd ddddd  kkkkkk ddddd                   f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


### C. Bus down

BEFORE (the banner replaces the rule and nothing else changes; `enter` still looks live):

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ Asks — open 30 (lab 30 · aruba 0)                 filter: owner all · age all  │
│ connecting to the bus…                                                         │
│   own   key      age  since header         title                               │
│ ❯ lab   i12      4d   T1    ☐ gh tokens     Revoke the two leaked GitHub CLI … │
│   lab   i16      4d   T1    ☐ Firefox test  Test the Firefox tab-close safety… │
│   lab   i17      4d   T1    ✔ Photo + Find  Two checks in the Direction app o… │
│   lab   vik1     4d   T295  ☐ Run the Cal…  Run the CalDAV spike (ac/infra#11… │
│   lab   i34      4d   T322  ! disk banner   Click Allow on one test banner at… │
│   lab   a1004-26 1d   T698  ☐ CPU levers    m4m has no idle CPU during builds… │
│   aruba a1005-01 1d   T803  ☐ uzi role      What role should uzi play in herd… │
│   lab   a1005-16 1d   T860  ✔ gmail Max     Login A (the gmail Claude Max sub… │
│   lab   a1005-18 15h  T883  ☐ CPA extras    After the v8 upgrade: do we add C… │
│                                                                                │
│                                                                                │
│                                                                                │
│────────────────────────────────────────────────────────────────────────────────│
│                                                                                │
│ enter answer  / text  a owner  g age  h answered  o webpage  q close           │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f bbbb b bbbb bb bbbb bb b bbbbb bb                 bbbbbbb bbbbb bbb b bbb bbb  f
f EEEEEEEEEE EE EEE EEEE                                                         f
f   DDD   DDD      DDD  DDDDD DDDDDD         DDDDD                               f
fRRRRRRRRRRRRRRRRRRRRRRRRRRRRR. .. ......     ...... ... ... ...... ...... ... . f
f   ...   ...      ..   ..    . ....... ....  .... ... ....... ......... ....... f
f   ...   ...      ..   ..    G ..... . ....  ... ...... .. ... ......... ... .. f
f   ...   ....     ..   ....  . ... ... ....  ... ... ...... ..... ............. f
f   ...   ...      ..   ....  Y .... ......   ..... ..... .. ... .... ...... ... f
f   ...   ........ ..   ....  . ... ......    ... ... .. .... ... ...... ....... f
f   ..... ........ ..   ....  . ... ....      .... .... ...... ... .... .. ..... f
f   ...   ........ ..   ....  G ..... ...     ..... . .... ..... ...... ... .... f
f   ...   ........ ...  ....  . ... ......    ..... ... .. ........ .. .. ... .. f
f                                                                                f
f                                                                                f
f                                                                                f
fDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDf
f                                                                                f
f bbbbb ......  b ....  b .....  b ...  b ........  b .......  b .....           f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


AFTER (red label in the rule, yellow status row, `enter answer` dimmed because saving cannot work):

```text
┌ popup ─────────────────────────────────────────────────────────────────────────┐
│ asks                                                                   30 open │
│ lab 30 · aruba 0 · owner all · age all                                         │
│ ▲ bus unreachable — read-only, retrying ────────────────────────────────────── │
│ ☐ gh tokens     Revoke the two leaked GitHub CLI tok…  lab   i12       4d   T1 │
│ ☐ Firefox test  Test the Firefox tab-close safety ch…  lab   i16       4d   T1 │
│ ✔ Photo + Find  Two checks in the Direction app on t…  lab   i17       4d   T1 │
│ ☐ Run the Cal…  Run the CalDAV spike (ac/infra#113):…  lab   vik1      4d T295 │
│ ! disk banner   Click Allow on one test banner at m4…  lab   i34       4d T322 │
│ ☐ CPU levers    m4m has no idle CPU during builds: w…  lab   a1004-26  1d T698 │
│ ☐ uzi role      What role should uzi play in herdr's…  aruba a1005-01  1d T803 │
│ ✔ gmail Max     Login A (the gmail Claude Max subscr…  lab   a1005-16  1d T860 │
│ ☐ CPA extras    After the v8 upgrade: do we add Code…  lab   a1005-18 15h T883 │
│                                                                                │
│                                                                                │
│                                                                                │
│                                                                                │
│ read-only while the bus is down — answers wait                                 │
│ enter answer  / text  a owner  g age  h answered  o webpage  q close           │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB                                                                   dd dddd f
f ddd dd d ddddd d d ddddd ddd d ddd ddd                                         f
f r rrr rrrrrrrrrrr r rrrrrrrrrr rrrrrrrr ssssssssssssssssssssssssssssssssssssss f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f . BBBBBBB BBBB  .... ... ....... ......... ...... ...  ddd   ddd       dd   dd f
f g ddddd d dddd  ddd dddddd dd ddd ddddddddd ddd dd dd  ddd   ddd       dd   dd f
f . BBB BBB BBBB  ... ... ...... ..... ................  ddd   dddd      dd dddd f
f y BBBB BBBBBB   ..... ..... .. ... .... ...... .. ...  ddd   ddd       dd dddd f
f . BBB BBBBBB    ... ... .. .... ... ...... ....... ..  ddd   dddddddd  dd dddd f
f . BBB BBBB      .... .... ...... ... .... .. ........  ddddd dddddddd  dd dddd f
f g ddddd ddd     ddddd d dddd ddddd dddddd ddd ddddddd  ddd   dddddddd  dd dddd f
f . BBB BBBBBB    ..... ... .. ........ .. .. ... .....  ddd   dddddddd ddd dddd f
f                                                                                f
f                                                                                f
f                                                                                f
f                                                                                f
f yyyyyyyyy yyyyy yyy yyy yy yyyy y yyyyyyy yyyy                                 f
f ddddd dddddd  k dddd  k ddddd  k ddd  k dddddddd  k ddddddd  k ddddd           f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


### D. Herdr: the PANES bar next to the NAVIGATE bar (`src/ui/display_panes.rs`)

BEFORE (two red chips, `PANES` and `VERSION`, and red keys; the NAVIGATE bar below it is blue):

```text
 PANES   window 316x55 · panes 284x54  1-3 focus  any key close   VERSION  0.8.2-ac-beta.145-dybala 
 NAVIGATE  esc back  ↑/↓ workspace  tab pane  ? keybinds                                            
```

```text
PPPPPPP  dddddd ...... d ddddd ......  ppp ddddd  ppp ppp ddddd  PPPPPPPPP ........................ 
AAAAAAAAAA ddd dddd  kkk ddddddddd  kkk dddd  k dddddddd                                            
```


AFTER (one red chip, bold red keys, `version` as a dim label; NAVIGATE below keeps its blue).
The SYNC chip (`SYNC 2 panes`, yellow `#ffd60a`) needs no mockup. It already is a single chip in
its own colour, and under the Mode colour rule it stays exactly as it is.

```text
 PANES   window 316x55 · panes 284x54  1-3 focus  any key close  version 0.8.2-ac-beta.145-dybala   
 NAVIGATE  esc back  ↑/↓ workspace  tab pane  ? keybinds                                            
```

```text
PPPPPPP  dddddd ...... d ddddd ......  ppp ddddd  ppp ppp ddddd  ddddddd ........................   
AAAAAAAAAA ddd dddd  kkk ddddddddd  kkk dddd  k dddddddd                                            
```


### E. Herdr: todos/notes panel (`src/ui/todo_board.rs`)

BEFORE (inner frame around the preview, count on the search row, primary button looks secondary):

```text
┌────────────────────────────────────────────────────────────────────────┐
│ todos/notes                                                            │
│ ┌────────────────────────────────────────────────────────────────────┐ │
│ │ left undone: ac/infra#94 Codex refresh test, due before 2026-10-03.│ │
│ │ On docker, read /opt/aiproxy/auths/codex-*.json …                  │ │
│ └────────────────────────────────────────────────────────────────────┘ │
│ / search todos                                               74 todos  │
│                                                                        │
│ infra · infra-helper                                                   │
│  ● left undone: ac/infra#94 Codex refresh test, due before 2026-10… #2 │
│  ● ac decides: Tailscale Services scope (ac/infra#166 c11854). HA … #5 │
│  ▼ context, NOT an ac decision: ac/infra#160 (status/ready). Remai… #1 │
│  ▲ your call: run the CalDAV spike — ac/infra#113 ⏎        → claude #3 │
│  ✓ CHECK 2026-09-28 ~00:05Z (before the 00:15Z GitLab backup): rus… #3 │
│                                                                        │
│             ↵ open pane    spc toggle    c clear done    esc close     │
└────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBBBBBBBBB                                                            f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f s dddd ddddddd ddddddddddd ddddd ddddddd ddddd ddd dddddd ddddddddddds f
f s dd ddddddd dddd ddddddddddddddddddddddddddddddd d                  s f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f d dddddd ddddd                                               dd ddddd  f
f                                                                        f
f kkkkk d mmmmmmmmmmmm                                                   f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f  y .. ........ ......... ........ ..... ............. ........ .. . .. f
f  k ........ ... .. .. ......... ............ ............... ...... .. f
f  r .... ..... ... ... ...... ..... . ............ .        d dddddd dd f
f  d xxxxx xxxxxxxxxx xxxxxxx xxxxxxx xxx xxxxxx xxxxxx xxxxxxxx xxxx dd f
f                                                                        f
f            UUUUUUUUUUUUU  UUUUUUUUUUUU  UUUUUUUUUUUUUU  UUUUUUUUUUU    f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


AFTER (no inner frame, count on the title row, search as the subtitle, preview under the list,
`↵ open pane` primary):

```text
┌────────────────────────────────────────────────────────────────────────┐
│ todos/notes                                                   74 todos │
│ / search todos                                                         │
│ ────────────────────────────────────────────────────────────────────── │
│ infra · infra-helper                                                   │
│  ● left undone: ac/infra#94 Codex refresh test, due before 2026-10… #2 │
│  ● ac decides: Tailscale Services scope (ac/infra#166 c11854). HA … #5 │
│  ▼ context, NOT an ac decision: ac/infra#160 (status/ready). Remai… #1 │
│  ▲ your call: run the CalDAV spike — ac/infra#113 ⏎        → claude #3 │
│  ✓ CHECK 2026-09-28 ~00:05Z (before the 00:15Z GitLab backup): rus… #3 │
│ ────────────────────────────────────────────────────────────────────── │
│ left undone: ac/infra#94 Codex refresh test, due before 2026-10-03.    │
│ On docker, read /opt/aiproxy/auths/codex-*.json …                      │
│                                                                        │
│             ↵ open pane    spc toggle    c clear done    esc close     │
└────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBBBBBBBBB                                                   dd ddddd f
f d dddddd ddddd                                                         f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f kkkkk d mmmmmmmmmmmm                                                   f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f  y .. ........ ......... ........ ..... ............. ........ .. . dd f
f  k ........ ... .. .. ......... ............ ............... ...... dd f
f  r .... ..... ... ... ...... ..... . ............ .        d dddddd dd f
f  d xxxxx xxxxxxxxxx xxxxxxx xxxxxxx xxx xxxxxx xxxxxx xxxxxxxx xxxx dd f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f dddd ddddddd ddddddddddd ddddd ddddddd ddddd ddd dddddd ddddddddddd    f
f dd ddddddd dddd ddddddddddddddddddddddddddddddd d                      f
f                                                                        f
f            AAAAAAAAAAAAA  UUUUUUUUUUUU  UUUUUUUUUUUUUU  UUUUUUUUUUU    f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```


## Risks / Trade-offs

- **Fixed 256 colours.** The asks pane does not follow herdr's theme. On a light theme or
  `theme = "terminal"`, 111/147/243 may read badly. That is acceptable while ac runs catppuccin.
  If that changes, herdr would need to pass its palette to popup programs (an env var), which is a
  herdr feature for later.
- **Grey labels are quieter.** `overlay0` labels on a black terminal are less bright than today's
  white ones. That is the point (keys stand out), and herdr's own bars do the same.
- **The asks tests read text.** Moving the columns changes the text the asks tests snapshot, so
  phase 2 rewrites those expectations. The rules that keep the program safe stay: the full-width
  repaint of every row (829ec84), cell widths via `cells()`/`clip`, typed answers, the bus-down
  read-only path, and the deleted-while-shown path.
- **Phase 3 lands after the 0.9.3 cutover.** `display_panes.rs` and `todo_board.rs` are fork
  files that the #171 port still has to move under `src/client/shell` (the 0.9.3 tree has neither
  screen yet), so fixing them now would only conflict with the port.

## Migration Plan

1. Phase 1 (this change): ac approves the mockups through herdr-helper.
2. Phase 2: `context` (or a worker it reviews) restyles `asks_tui.py` on CONTEXT branch
   `feat/asks-tui-herdr-style`, made with
   `wt switch --create feat/asks-tui-herdr-style --base main --no-cd`. `context` reviews and merges.
   Rollback = do not merge; the popup keeps running the old file.
3. Phase 3, after the 0.9.3 cutover, with ac's OK: the PANES bar and the todos/notes panel in
   herdr. Each lands as a separate commit on its own fork issue.

## Open Questions

- None blocking. One idea for later: herdr's popup frame could show the program's name (`asks`)
  instead of `popup`, so the program would not need its own title row. That is a herdr change and
  is left out of this plan.
