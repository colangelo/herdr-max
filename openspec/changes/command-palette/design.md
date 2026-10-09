## Context

ac wants one bar to find and run any herdr command, and to send a message to a session without
switching to it (issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/182). It is
built after the #171 cutover, on the 0.9.3 client shell (`src/client/shell`).

Facts this design rests on, checked on `sync/merge` 611ae906:

- **The command list.** `src/input/keybind_help.rs` builds the help panel from `keybind_help_groups`:
  about 120 `(key label, description)` entries in groups (global, navigation, workspaces / tabs,
  panes, …). An entry knows its key and its words, not its action. Most entries map one-to-one to
  a `KeybindAction` (79 variants, `src/input/keybindings.rs`); a few are not commands (`esc back`,
  `1..9 switch workspace`).
- **Running a command.** `ClientShellState::record_binding(KeybindMatch::Action(action), outcome)`
  (`src/client/shell/actions.rs`) runs any action; mouse clicks already call it that way
  (`mouse.rs`, `mobile.rs`). So the palette needs no second dispatcher.
- **Overlays on 0.9.3.** AGENTS.md on this tree: an overlay is a `ClientShellOverlay` variant
  (`state.rs`), rendered in `overlays.rs`, keyed in `overlay_input.rs`, with its chords in
  `help_lines` and a rendered-layout test under `src/client/shell/tests/`. The pre-port kit names in
  the brief (`AnchoredPanelSpec`, `ButtonRow`, `ListCursor`, the `overlays!` list) do not exist in
  the 0.9.3 client shell. Their 0.9.3 equivalents are `TextEditor`, `move_picker`'s `list_chord`,
  `matches_query` and `render_search_row`, and the panel shell `render_panel_shell`; the palette
  uses those.
- **Typing into a pane.** `pane.send_text` (`PaneSendTextParams { pane_id, text }`) types text;
  `pane.send_keys` presses keys; `agent.prompt` sends a prompt with bracketed paste. Chunking exists
  only in the CLI (`herdr pane send-text --chunk BYTES --chunk-delay MS`, `src/cli/pane.rs`).
  Claude Code collapses one large paste into `[Pasted text #N]`; text sent in ~300-byte pieces with
  the default gap arrives as typed text; the final Enter must be its own key (CONTEXT
  `SKILLS/herdr-compact/SKILL.md` § "Why pieces").
- **Target state.** `agent_status` (`idle | working | blocked | done | unknown`), `blocked_reason`
  (`question | permission | form | other`) and `blocked_since` are on every agent and pane
  (`src/api/schema/agents.rs`, `panes.rs`); `events.subscribe` streams changes.
- **Other Macs.** The 0.9.3 client can hold several endpoints (`ClientShellEndpoint`,
  `ClientEndpointId`), so a client connected to the other Mac's server sees its panes too.
- **agent-bell** lives in `ac/infra` at `tools/agent-bell` (the CLI ships in its plugin; CONTEXT
  skills use it). `agent-bell who` prints `name  host.sid  cwd  agent  beat  source` per live
  session; `send --to NAME|HOST.SID [--queue|--interrupt] [--from NAME] TEXT` (wake is the
  default); `status MSG_ID` returns `queued | held (+ note) | injected | unknown`. A peer message is
  framed "not from the user … does not carry the user's approval". A sender with no session id
  gets no pushed receipt and polls `status`. A `held` receiver must not get a fallback
  SendMessage.
- **Hermes.** 0.9.3 ships `src/detect/manifests/hermes.toml`: blockers for a dangerous-command
  approval, a clarification prompt, a credential prompt and a confirmation; none sets
  `blocked_reason`, so a blocked hermes pane reports `other`.

## Goals / Non-Goals

**Goals:**

- One key opens a bar that lists every herdr command with its shortcut; typing filters; `enter` runs
  it exactly as its key does (part 1, shaped to go upstream).
- Send text to a session, as ac or as a note, seeing the target's state first, with a busy policy
  and a receipt that says what it means (part 2, fork only).
- One server method for sending, so the CLI, the palette and a later phone client share it.

**Non-Goals:**

- The phone/iPad client itself (part 3; planner owns that research).
- Running arbitrary shell commands from the bar.
- Matched-letter highlighting and recent-command ranking (later polish).
- Testing hermes end to end (a later task, 3.x below).

## Decisions

1. **One command table, two readers.** `keybind_help.rs` gets a table of
   `CommandEntry { group, label, action: Option<KeybindAction>, key_label }`; `keybind_help_groups`
   becomes a projection of it (same output, so the help tests stay), and the palette reads the
   same table, keeping the entries with an action. A new action that is in help is in the palette
   by construction. *Instead:* a second list in the palette, which would drift.
2. **Running goes through `record_binding`.** The palette closes, then calls
   `record_binding(KeybindMatch::Action(action))`, so a command run from the bar behaves exactly as
   its key (including the prompts some actions open). Indexed actions (`switch workspace 1-9`,
   `focus agent 1-9`) show once, with a digit argument typed after them (`switch workspace 3`).
3. **Client-shell overlay, near the top.** `ClientShellOverlay::Palette(PaletteOverlay)`: a
   `TextEditor` query, the filtered rows, a selection index, a scroll. 82 columns wide (clamped to
   the screen), anchored a few rows from the top, the list grows to at most 16 rows. Keys:
   `list_chord` for movement (↑↓, ^j/^k, ^d/^u, home/end), `enter` run, `tab` complete the selected
   command's words into the query, `esc` clears then closes. Fuzzy matching is subsequence over
   `group + " " + label`, scored by contiguous runs and word starts; ties keep the help order.
4. **Its own key.** New `KeybindAction::CommandPalette`, default `prefix+:` (open question 1), with a
   help entry in `global`, and `:` added to the NAVIGATE bar's hints.
5. **Send is a server method: `agent.send`.** Per AGENTS.md's runtime/client boundary, sending text
   to a session, the target's state and the receipt are runtime facts, so they live in the server
   and the API. The TUI owns only the bar, its layout and its keys.

   ```text
   agent.send { target, text, mode: "typed" | "note", busy: "queue" | "interrupt",
                answer: Option<String> }
     -> AgentSent { delivery: "typed" | "note", pieces, enter_sent, note: Option<NoteReceipt> }
   event AgentSendReceipt { target, msg_id, state: "queued" | "held" | "injected", note }
   ```

   - **typed** (as me): the server splits `text` into ~300-byte pieces on character boundaries,
     sends them with `pane.send_text`'s path and the CLI's default gap, then presses Enter as its
     own key. How a newline inside the text reaches Claude Code's input (a line break, not a
     submit) is not verified; task 2.2 checks it live before multi-line sends are offered, and
     until then the bar sends one line.
   - **note**: the server runs `[palette.send] note_command` (default `agent-bell`) as
     `send --to <name> --from herdr-palette [--interrupt] <text>` and parses its JSON; it then polls
     `status <msg_id>` until `injected` or 60 s, emitting `AgentSendReceipt` each change.
   - `answer` (a blocked target): the server presses that option's key with `pane.send_keys`
     instead of typing text.
   - A CLI `herdr agent send <target> [--note] [--interrupt] <text>` uses the same method.
   *Instead:* a TUI-only send through the private client socket, which AGENTS.md rules out, and
   which the phone could not use.
6. **Names: herdr's agents first, then agent-bell's.** The target list merges, per endpoint, the
   agents of every connected herdr server (name, pane, state; `as me` possible) with
   `directory_command` (default `agent-bell who`) rows, joined on the Claude session id
   (`agent_session.value` = the `sid` in `host.sid`). A session in both is one row with herdr's
   state. A session only in agent-bell is `note` only. The directory is read by the server on
   demand (`agent.directory`, cached 10 s), not by the client.
7. **Blocked targets get options, not free text.** When the target is `blocked` with reason
   `question`, `permission` or `form`, the server reads the prompt's visible options from the
   pane's detection text (a per-agent extractor, Claude Code first: numbered `1. … 2. …` rows in the
   bottom buffer) and returns them on `agent.get` as `prompt_options`. The bar lists them; picking
   one sends `agent.send { answer }`. With reason `other`, or no options found, the bar offers only
   `open pane`. Free text is never typed into a blocked target from the bar.
8. **Busy targets queue by default.** `working` → `busy: queue` types the text now (Claude Code holds
   typed text until its next pause). `alt+enter` → `busy: interrupt`: the server presses `esc`,
   waits up to 3 s for the status to leave `working`, then types; if it does not, it sends nothing
   and says so (open question 3).
9. **Hermes.** A hermes pane is an ordinary herdr agent target: `as me` types into it when it is
   `idle`, `done` or `working` (queue). Every hermes blocker reports `blocked_reason: other`, so a
   blocked hermes pane is offered `open pane` only. That matters most for its credential prompt,
   where typed text would be a secret typed in the wrong place. A later task maps hermes's
   blockers to reasons (approval → `permission`, clarification → `question`) in its manifest and
   then tests the palette against a live hermes (tasks 3.2, 3.3).
10. **Receipts say what they are.** As me: `AgentSent` (pieces, Enter pressed), then the first state
    change the client sees for that pane within 5 s. As a note: agent-bell's states, with its
    `held` note verbatim ("held until morning: if urgent, tell your gestore"). Every receipt ends
    with "This means it arrived, not that it was read or done." A `held` note never triggers a
    second delivery path.
11. **The master key stays at the keyboard.** `as me` is only offered for panes on servers this
    client is attached to, which already means ac's keyboard and ac's tailnet. A phone path (part 3)
    must add device identity before it can call `agent.send`; until then `agent.send` accepts
    calls only on the local socket, like every other method today.
12. **Part 3 shape.** The phone path calls the same `agent.send`, `agent.get` (with
    `prompt_options`) and `events.subscribe`. If ac picks the Collie pilot, its compatibility with
    the 0.9.3 protocol is checked first (planner's note on #182); nothing in this change depends on
    that choice.

### What agent-bell would need (for its owner, `ac/infra`)

- `who --json` (one object per session) so herdr does not parse columns.
- `send --json` printing `{msg_id, held, note}` so the receipt starts without a second call
  (the README shows a JSON send result; confirm it is stable).
- A sender with no session: `--from herdr-palette` without a `from_sid`, and confirmation that
  `status MSG_ID` is the supported way to follow it (no pushed receipt is owed to it).
- No change to the framing: a palette note is a peer message and must stay "not from the user".

## Mockups

All mockups are 80 cells inside herdr's frame (82 with it), drawn from the 0.9.3 overlay idiom and
`docs/ui-style.md`. Names, counts and texts are made up. Each mockup is a text block and its paint
map, one code per cell. Paint codes (the unify-tui-look set): `B` bold text · `.` text · `d` dim
(`overlay0`) · `k` key or heading (bold `accent`) · `A` selection bar or primary chip (`accent`
fill, dark bold text) · `U` secondary chip (`surface0` fill) · `g` green · `y` yellow · `r` red ·
`m` mauve (a name) · `s` rule (`surface1`) · `f` herdr's frame.

### A1. Closed: how you find it

The NAVIGATE bar (prefix pressed) gains one hint, `: commands`. Nothing else changes; with the bar closed the palette costs nothing.

```text
 NAVIGATE  esc back  : commands  ↑ / ↓ ws  ⇥ pane  g navigator  c new tab
```

```text
AAAAAAAAAA kkk dddd  k dddddddd  k k k dd  k dddd  k ddddddddd  k ddd ddd
```

### A2. Open, nothing typed

Centred near the top, 82 wide. Title row with the count on the right; the input row is the subtitle; then a rule. Groups are the help panel's groups. The selected row is the accent bar. `send to a session…` is the way into part 2 for someone who does not know the word. Shortcuts are dim and right-aligned, as in the help panel.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ commands                                                           94 commands │
│ : type a command, or send <session> <text>                                     │
│ ────────────────────────────────────────────────────────────────────────────── │
│  global                                                                        │
│   keybinds                                                            prefix+? │
│   settings                                                            prefix+s │
│   notification center                                            prefix+ctrl+n │
│                                                                                │
│  sessions                                                                      │
│   send to a session…                                                      send │
│                                                                                │
│  panes                                                                         │
│   split vertical                                                      prefix+v │
│   zoom pane                                                           prefix+z │
│   sync panes                                                    prefix+shift+s │
│  ↓ 80 more                                                                     │
│                                                                                │
│ enter run  ↑↓ ^j/^k move  tab complete  esc close                              │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBBBBBB                                                           dd dddddddd f
f k dddd d dddddddd dd dddd ddddddddd dddddd                                     f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f  kkkkkk                                                                        f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f   ........                                                            dddddddd f
f   ............ ......                                            ddddddddddddd f
f                                                                                f
f  kkkkkkkk                                                                      f
f   .... .. . ........                                                      dddd f
f                                                                                f
f  kkkkk                                                                         f
f   ..... ........                                                      dddddddd f
f   .... ....                                                           dddddddd f
f   .... .....                                                    dddddddddddddd f
f  d dd dddd                                                                     f
f                                                                                f
f kkkkk ddd  kk kkkkk dddd  kkk dddddddd  kkk ddddd                              f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

### B. Filtered, with shortcuts

Fuzzy match on the label and the group, best first; matched letters are not highlighted in v1 (a later polish). The group moves into a dim middle column once the list is flat. `enter` runs the bar's row and closes the palette; `esc` clears the query, a second `esc` closes.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ commands                                                               4 of 94 │
│ : spl▏                                                                         │
│ ────────────────────────────────────────────────────────────────────────────── │
│   split vertical                        panes                         prefix+v │
│   split horizontal                      panes                     prefix+minus │
│   break pane to new tab                 panes                         prefix+! │
│   display pane labels                   panes                         prefix+i │
│                                                                                │
│ enter run  ↑↓ ^j/^k move  esc clear                                            │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBBBBBB                                                               d dd dd f
f k ...k                                                                         f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f   ..... ..........                      ddddd                     dddddddddddd f
f   ..... .... .. ... ...                 ddddd                         dddddddd f
f   ....... .... ......                   ddddd                         dddddddd f
f                                                                                f
f kkkkk ddd  kk kkkkk dddd  kkk ddddd                                            f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

### C1. Send: pick the target

`send ` switches the list to sessions. A session herdr can type into (its pane is on a connected server) says `as me`; one only agent-bell knows says `note`. State comes from herdr (`agent_status`) for its own panes; agent-bell targets show `—` (agent-bell has no state). Dot colours are the state colours: green idle, yellow working, red blocked, dim unknown.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ send · pick a session                                              12 sessions │
│ : send pla▏                                                                    │
│ ────────────────────────────────────────────────────────────────────────────── │
│   ● planner               idle      this Mac · w2:p1                     as me │
│   ● planner-hlp           working   this Mac · w2:p3                     as me │
│   ● planner-aruba         —         ac-mbm5 · agent-bell                  note │
│                                                                                │
│ enter pick  ↑↓ move  esc back                                                  │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB d dddd d ddddddd                                              dd dddddddd f
f k .... ...k                                                                    f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f   y ...........           yyyyyyy   dddd ddd d ddddd                     dd dd f
f   d .............         d         ddddddd d dddddddddd                  dddd f
f                                                                                f
f kkkkk dddd  kk dddd  kkk dddd                                                  f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

### C2. Send: write it and choose the mode

The title names the target (mauve, like a name next to a heading) and its state. One line says what the mode means, every time: this is the master key, so the bar never lets it be ambiguous. `tab` flips the mode; the primary chip follows. One line of text in v1 (decision 5).

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ send to planner                                        idle · this Mac · w2:p1 │
│ > please file the inbox item about the palette as done▏                        │
│ ────────────────────────────────────────────────────────────────────────────── │
│ as me:  herdr types this into planner's pane, as if you were there.            │
│         It can approve and decide with your authority.                         │
│                                                                                │
│                  ↵ send as me    tab as a note    esc cancel                   │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB BB mmmmmmm                                        dddd d dddd ddd d ddddd f
f k ...... .... ... ..... .... ..... ... ....... .. ....k                        f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f BB BBB  ddddd ddddd dddd dddd ddddddddd ddddd dd dd ddd dddd dddddd            f
f         dd ddd ddddddd ddd dddddd dddd dddd dddddddddd                         f
f                                                                                f
f                 AAAAAAAAAAAAAA  UUUUUUUUUUUUUUU  UUUUUUUUUUUU                  f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

### C3. Send: the receipt

As me: herdr's ack (`AgentSent`), then the first state change it sees within a few seconds. As a note: agent-bell's `queued`, `held` (with its note, "held until morning") or `injected`. The last line is always there.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ sent to planner                                                          as me │
│ ────────────────────────────────────────────────────────────────────────────── │
│ ✔ typed into w2:p1 (52 characters, 1 piece), enter pressed                     │
│ ● planner is working                                                           │
│                                                                                │
│ This means it arrived, not that it was read or done.                           │
│                                                                                │
│                            ↵ open pane    esc close                            │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB BB mmmmmmm                                                          dd dd f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f g ggggg gggg ggggg ggg ggggggggggg g ggggggg ggggg ggggggg                     f
f y yyyyyyy yy yyyyyyy                                                           f
f                                                                                f
f dddd ddddd dd dddddddd ddd dddd dd ddd dddd dd ddddd                           f
f                                                                                f
f                           AAAAAAAAAAAAA  UUUUUUUUUUU                           f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

### D. A blocked target: its prompt's options, not free text

When the target is `blocked` with a reason herdr understands (Claude Code's `permission`, `question`, `form`), the bar reads the prompt's visible options from the pane (the same detection text `agent read` uses) and offers them; picking one sends that key. Free text is not offered: typed text would land in the prompt. A blocked target with reason `other` (every hermes blocker today, including its credential prompt) offers only `open pane`.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ send to infra-helper                               blocked: permission · w1:p4 │
│ waiting: Bash(ssh docker …) — allow this command?                              │
│ ────────────────────────────────────────────────────────────────────────────── │
│   1  Yes                                                                       │
│   2  Yes, and don't ask again for ssh docker                                   │
│   3  No, and tell Claude what to do differently                                │
│                                                                                │
│ Picking one presses that key in its pane, as you would.                        │
│                                                                                │
│                     ↵ answer    o open pane    esc cancel                      │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB BB mmmmmmmmmmmm                               dddddddd dddddddddd d ddddd f
f dddddddd ........ ...... .. . ..... .... ........                              f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
fAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAf
f   k  .... ... ..... ... ..... ... ... ......                                   f
f   k  ... ... .... ...... .... .. .. ...........                                f
f                                                                                f
f ddddddd ddd ddddddd dddd ddd dd ddd ddddd dd ddd dddddd                        f
f                                                                                f
f                    AAAAAAAAAA  UUUUUUUUUUUUU  UUUUUUUUUUUU                     f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

### E. A busy target: queue or interrupt

Default queue (open question 3). Interrupt sends `esc`, waits for the agent to leave `working` (bounded, 3 s), then types the text; if it stays working the bar says so and sends nothing.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│ send to planner-hlp                                            working · w2:p3 │
│ > also check the second inbox▏                                                 │
│ ────────────────────────────────────────────────────────────────────────────── │
│ busy: it is in the middle of a turn.                                           │
│ enter queues it: Claude Code takes it at its next pause.                       │
│ alt+enter interrupts first (esc), then sends.                                  │
│                                                                                │
│           ↵ queue    alt+↵ interrupt    tab as a note    esc cancel            │
└────────────────────────────────────────────────────────────────────────────────┘
```

```text
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
f BBBB BB mmmmmmmmmmm                                            ddddddd d ddddd f
f k .... ..... ... ...... .....k                                                 f
f ssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssssss f
f yyyyy dd dd dd ddd dddddd dd d ddddd                                           f
f kkkkk dddddd ddd dddddd dddd ddddd dd dd ddd dddd dddddd                       f
f kkkkkkkkk dddddddddd ddddd dddddd dddd dddddd                                  f
f                                                                                f
f          AAAAAAAAA  UUUUUUUUUUUUUUUUU  UUUUUUUUUUUUUUU  UUUUUUUUUUUU           f
ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
```

## Risks / Trade-offs

- **Typing as ac is a master key.** Many sessions run in bypass mode. Mitigation: the mode line on
  every send (C2), no free text into blocked targets (D), local socket only (decision 11), and
  cross-Mac `as me` out of v1 if ac picks 4b.
- **Prompt options are read from the screen.** The extractor can miss or misread a prompt.
  Mitigation: options are offered only when the extractor is sure (numbered rows under a known
  blocker), and `open pane` is always there.
- **Chunked typing is slower.** A 3 kB message is ten pieces, about a second. Acceptable for a
  message; the receipt shows the piece count.
- **agent-bell is outside herdr.** The note path depends on a configured command. Mitigation: it is
  config (`note_command`), the bar shows `note` targets only when `directory_command` answers, and
  herdr upstream never sees it (part 2 is fork only).
- **Upstream fit.** Part 1 touches the help registry and adds one overlay; it is shaped as its own
  commits so it can be offered upstream. Part 2 stays in the fork.

## Migration Plan

Nothing to migrate: a new action with a default key (users with `prefix+:` bound keep their
binding, and the palette is then unbound and listed as `unset` in help), a new overlay, a new API
method (additive; the endpoint shape digest and the schema artifact move together), new optional
config. Ship part 1, then part 2, each behind its own commits and tests.

## Open Questions

The four questions for ac are in `proposal.md` § "Open questions for ac": the key, the default mode,
the busy policy, and cross-Mac `as me` in v1. Technical ones, decided here unless someone objects:
the server reads `agent-bell who` (not the client); a note is sent `--from herdr-palette`; the
interrupt wait is 3 s.
