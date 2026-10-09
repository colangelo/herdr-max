## Context

Findings for issue 172, 2026-10-08. Sources: the fork's tree on `port/chrome`
(`src/client/shell/state_presentation.rs`, `activity.rs`, `agent_sidebar.rs`),
`src/detect/manifests/claude.toml`, `src/integration/assets/claude/herdr-agent-state.sh`,
and Claude Code's public docs (hooks, statusline, sub-agents, interactive mode).

### What herdr does today

- Detection is screen-based. Rules whose id starts with `background_` mark a
  Working pane as *background work* (`is_background_work_rule`, `manifest.rs`).
  Claude has `background_agents_working` ("Waiting for N background agents to
  finish") and `background_mcp_task_working` ("... · N MCP tasks still
  running"). A background *shell* has no rule of its own today.
- The answer is a `bool` all the way down: `AgentDetection.background_work` ->
  `TerminalState::background_work()` -> `resource_facts.background_activity`
  (`BTreeMap<String, bool>`) -> `AgentRow.background_work` in the sidebar.
- The glyph: `StatePresentation::agent_icon`. With the spinner on, a background
  row shows `state_symbols.background` (default `■`) and, for half the cycle,
  `background_alt` (default `◆`); ac sets `·` / `●`. The phase is
  `(spinner_frame / 8) % 2`. The spinner ticks every
  `ui.status_spinner_ms` (default 200 ms), so each phase lasts 8 ticks = 1.6 s
  and a full pulse is 3.2 s. That is "the current speed".
- Shells, subagents, workflows and MCP tasks all draw the same pulse today. The
  icon says "something runs", never "what" or "how many".

### What Claude Code offers (docs; live capture still to do, task 0.1)

- On screen: a panel below the prompt for background subagents (rows, `(+N)`
  for nested ones, `/tasks` to manage), the footer text the two rules above
  match, and a shells/tasks surface behind `/tasks` and `Ctrl+B`.
- Hooks: `SubagentStart` / `SubagentStop` carry `agent_id` and `agent_type`.
  There is no shell or workflow event. The best source is **`background_tasks`
  on `Stop` and `SubagentStop` input**: entries with `id`, `type` (`shell`,
  `subagent`, `monitor`, `workflow`, `teammate`, `cloud session`, `MCP task`),
  `status`, `description`. It is a snapshot at turn end, not a live query.
- Statusline JSON has no background count. `subagentStatusLine` gets a
  `tasks[]` list, subagents only, and only while the agent panel shows.
- herdr's own Claude hook asset exits early for subagent calls and drops
  `SubagentStop` on purpose (it once revived idle panes), so counting through
  hooks is new work with a real regression risk. It stays out of phase 1.

## Goals / Non-Goals

**Goals**
- One config switch, two styles, `frames` unchanged.
- A count of 1..8 in `braille`, never wrong in the direction of "nothing runs"
  while work runs.
- Same speed, same colour (`state_colors.background`), same one-cell width.

**Non-Goals**
- Telling shells from subagents from workflows by glyph. Counts only; kinds can
  be a later `background_mark` value.
- Changing when a pane counts as Working. This is presentation only.
- Hook-driven state. Hooks may feed the count later; they never own the state.

## Decisions

### Config shape

`[ui] background_mark = "frames" | "braille"` (a bad value is a config parse error like every other `[ui]` enum), an enum
`BackgroundMarkConfig { Frames (default), Braille }`, `serde(rename_all =
"lowercase")`, applied by config reload (no restart), next to `state_symbols`.
It lives in `[ui]`, not in `[ui.state_symbols]`, because it picks a *style*;
`state_symbols` stay per-glyph overrides.

Interaction with `state_symbols`:

| `background_mark` | frame A (rest) | frame B |
|---|---|---|
| `frames` | `background` (default `■`) | `background_alt` (default `◆`) |
| `braille` | `background` (default `·` in this mode) | braille cell for the count; `background_alt` ignored |

`braille` defaults frame A to `·` because that is the small central dot ac
asked for; an explicit `state_symbols.background` still wins.

### Glyph table (Braille Patterns, U+2800 block, one cell wide)

| count | glyph | dots |
|---|---|---|
| 1 | `⠁` | 1 |
| 2 | `⠃` | 1 2 |
| 3 | `⠇` | 1 2 3 |
| 4 | `⡇` | 1 2 3 7 |
| 5 | `⡏` | 1 2 3 4 7 |
| 6 | `⡟` | 1 2 3 4 5 7 |
| 7 | `⡿` | 1 2 3 4 5 6 7 |
| 8+ | `⣿` | all eight |

A `const [&str; 8]`; index `min(count, 8) - 1`. A count of 0 never reaches the
renderer (the row is not background work). The working spinner uses braille
frames too (`⠋⠙⠹...`), but those are never a single dot or a left-filling
stack, and the background row alternates with `·`, so the two cannot be
confused; the colour (`state_colors.background`) also differs.

### Alternation timing

Reuse the existing phase `(spinner_frame / 8) % 2`: no new timer, no new
wake-up, same 1.6 s per frame at the default 200 ms and it follows
`ui.status_spinner_ms`. `status_spinner = "off"` keeps today's behaviour: the
static background glyph (frame A).

### Where the count comes from

1. **Screen (phase 1).** `claude.toml` keeps the rule ids; the two background
   rules get a capture of the number (`Waiting for (\d+) background agents`,
   `(\d+) MCP tasks? still running`). `AgentDetection.background_count: u8`
   (saturating at 255) is set from the capture; a background rule with no
   number, and any future shell rule, counts as 1. If several background rules
   could match one screen, the rule that matched (highest priority) wins; counts
   are not summed across rules in phase 1, so a screen showing "2 agents" and
   "1 MCP task" reads 2, an undercount that is documented rather than guessed.
2. **Hook snapshot (phase 2, optional).** The Claude `Stop` hook input has
   `background_tasks`; reporting its length as `background_count` would give an
   exact figure across kinds. It is a turn-end snapshot, so it can only *raise*
   confidence for a pane the screen already says is background-working; it
   never makes a pane Working. Decided with a live capture and ac's OK, because
   of the hook history in `claude-attention-hint/proposal.md`.

### Plumbing

- `TerminalState::background_count() -> u8` next to `background_work()`;
  requires the live state to be Working, like the flag, so a stale value never
  reaches a row.
- `ClientShellResourceFacts.background_count: Option<BTreeMap<String, u8>>`,
  `serde(default, skip_serializing_if = "Option::is_none")`. Old clients ignore
  it, old servers omit it; the client then falls back to 1 when
  `background_activity` is true. `background_activity` stays, so nothing else
  changes.
- `AgentRow.background_count: u8` beside `background_work`;
  `StatePresentation::agent_icon` takes the count and the mark style and
  returns `&str` from the const table. Style and symbols are resolved once in
  `StatePresentation::from_config`, not in the row loop.

### Performance

Per row, per render: one `u8` read and one array index. No allocation, no
string formatting, no new lock. Detection adds one capture group on a rule that
already runs on every screen snapshot. Rendered-layout cost is unchanged, so no
scaling bench is needed; the sidebar snapshot test covers 1 and 15 rows.

## Risks / Trade-offs

- The screen count is only as good as Claude's footer text, which changes
  between versions. Mitigation: no number -> 1 dot; the rule ids and the
  `claude.toml` version bump are the only places that move.
- Undercount when two background rules match one screen (see above). Accepted
  for phase 1; phase 2 fixes it exactly.
- Braille glyph width in some fonts. Mitigation: `is_single_cell` validation
  already guards user overrides; the table is checked by a unit test with
  `unicode_width`.
- Other agents (codex, pi, ...) have no background rule, so they never show the
  mark. Unchanged.

## Migration Plan

Default `frames` changes nothing. ac tests by setting `background_mark =
"braille"` in `~/.config/herdr/config.toml` and reloading. Rollback is deleting
the line.

## Decision (ac, 2026-10-09)

The open question, whether a Claude waiting at its prompt with background work should read idle,
was decided by ac for option (a): *"a"* (idle plus the braille mark). Status follows upstream
(987b070f): a Claude waiting at its prompt reads idle or done even while background shells or
agents run, and the background work is shown only by the mark. This is the real fix for fork
issue 78. Consequences built here:

- the fork's `background_shell_working` rule is gone from `claude.toml` again (both copies); the
  upstream rules `background_agents_working` ("Waiting for N background agents to finish") and
  `background_mcp_task_working` are upstream's and stay;
- the count is read from the footer by `detect::background_count`, independent of the status rule;
- in `frames` mode (the default) an idle row with shells looks like any idle row, as upstream; the
  pulse still shows for the upstream rules that hold a pane Working;
- the permission-dialog `not` guard on the old rule goes with it (nothing outranks the dialog
  rules any more).

## Open Questions

- Names: `frames`/`braille` (as asked) versus `pulse`/`count`. Kept ac's words.
- Phase 2: the `Stop` hook's `background_tasks` as an exact count across kinds, only with ac's OK.
