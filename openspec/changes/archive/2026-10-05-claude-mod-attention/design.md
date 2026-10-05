## Context

Today a Claude pane's `blocked` state and its reason (`question`, `permission`,
`form`, `other`) come from the screen: the detector reads the bottom buffer and
`src/detect/manifests/claude.toml` rules say what is visible. The reason is
carried by the detection result (`AgentDetection.blocked_reason`), published by
`decide_screen_detection_publish` in `src/pane/agent_detection.rs`, and held on
`TerminalState` as a `blocked_spell`. Hook-driven reports exist
(`pane.report_agent`, with an authority model) but upstream dropped them for
Claude's blocked state in `295b09ca`, because a settings hook cannot see a
dialog end by Esc.

A Claude Code mod (Claude Code 2.1.287 or later) is a plugin of JS handlers
that run inside Claude Code as middleware around `next(e)`. What the spike
established and what the reference says (https://code.claude.com/docs/en/plugins/mods/events,
read again for this design):

- `tool.call` wraps the call: `await next(e)` returns when the call settles, but
  does **not** settle on Esc; `next.signal` aborts instead. The spike's
  `finally` plus an abort listener closes in about 1 s on answer, "Chat about
  this", Esc and SIGTERM.
- `tool.check` "fires after the permission rules and the settings hooks have
  decided, and `next(e)` resolves to their decision: `allow`, `ask`, or `deny`."
  When the decision is `ask`, Claude Code shows the permission prompt next. The
  hook sees the **decision**, not the prompt: it does not run again when the
  prompt is answered. `tool.call`'s `await next(e)` spans the permission check
  and then the tool, so it settles only when the tool has finished, which for a
  long Bash command is long after the prompt ended.
- `tool_use_id` "is the same at every event of the call", so `tool.check` and
  `tool.call` for one call can be correlated.
- `turn.complete` fires for an interrupted turn too; `session.end` fires on
  exit, `/clear` and `/resume`.
- Settings-hook events are mod events too (`classic.PermissionRequest`,
  `classic.PermissionDenied`, `classic.PostToolUse`), and `ui.render` can
  observe render sites such as `ToolUse` and `ToolProgress` by `requestId`
  (the tool call id).
- A hook gets 10 s of its own time; `$.process.run` takes 30 s by default;
  `$.clock.every` runs work on a timer; `$.env.get` is async; a mod's
  `$.ui.log` lines show in the transcript.

## Decisions

### (a) The report surface: a small neutral API, not a display token

A new method `pane.report_hint`:

```
pane.report_hint {
  pane_id, source, agent,
  kind: "question" | "permission",     // or absent with clear: true
  id?: string,                         // the source's own id for the dialog
  ttl_ms?: u64,                        // default 15000, at most 60000
  clear?: bool,
  seq?: u64                            // per source, wall-clock based; older is ignored
}
```

`seq` is the sender's wall clock in microseconds, built as `Date.now() * 1000 +
(counter % 1000)`, so two reports in the same millisecond still order. It is
deliberately not a counter that starts at 1: `/reload-plugins` or a plugin
update restarts the mod's module (and its counter) while the Claude process and
its live hint carry on, and a restarted counter would be lower than the live
hint's `seq`, so herdr would ignore the new mod until it caught up. A clock
that keeps rising across reloads has no such window. (A backwards wall-clock
step can still make one report stale; the next heartbeat, at most 5 s later,
carries a fresh `seq` and the TTL covers the gap.)

CLI `herdr pane report-hint <pane> --source S --agent claude --kind question
[--id X] [--ttl-ms N]` and `--clear`. The response is the usual pane info.

Why not the spike's display token (`pane report-metadata --token question=open:...`):
a token is display-only metadata with a free-form string value and the TTL the
caller picks. Making the detector read a display token would give a presentation
string a runtime meaning, which the runtime/client guardrail in `AGENTS.md`
warns against, and the detector would have to parse `open:<id>` out of it. A
hint is a typed runtime fact (a kind, an id, a freshness), named for what it is
and not for where it is drawn. Why not `pane.report_agent`: it carries the
whole agent state (working, idle, blocked) with authority over the screen, which
is exactly the hook-driven state upstream removed. A hint can only say "blocked
on X", never "idle" or "working", so it cannot recreate that failure.

Storage: `TerminalState.agent_hint: Option<AgentHint { kind, id, source, agent,
seq, expires_at }>`. Runtime only: not in the session snapshot, not carried
through a live handoff (a hint is a few seconds old at most, and the mod
re-reports within its heartbeat). One hint per terminal; a newer `seq` from the
same source replaces it, a different source's hint is ignored while another's is
live (the first source wins until it clears or ages out). A hint is dropped when
the pane's agent changes, the process exits, or the pane closes.

### (b) How the detector merges the hint with the screen, and age-out

The effective blocked state is computed where the screen result is published:

1. **No live hint:** exactly today's behaviour. No hint code runs per frame.
2. **A live `question` hint** (fresh, and the pane's agent is the hint's agent):
   the pane is `blocked` with reason `question`, **regardless of the screen**.
   The screen may read idle or working (the dialog is half drawn, or Claude
   shows a spinner behind it) and it cannot clear the hint. Only the mod's
   close, the TTL, or the agent leaving can. This is the whole point: the mod's
   close is exact where the screen was not.
3. **A live `permission` hint:** the pane is `blocked` with reason `permission`
   from the moment the hint arrives, but **the screen may clear it**: when the
   screen detection has read something other than `blocked` continuously for
   0.5 s after it has shown a blocked prompt, the hint is dropped. (It was 1.5 s in
   the first draft; the proof run showed the screen alone leaves blocked within
   about 0.4 s of an allow, and 1.5 s made the hint hold the pane blocked 1.4 s
   longer than the screen would, so it is about two detection polls.) If the screen
   has not yet shown one (the mod reports a few milliseconds before the dialog
   is drawn and detection polls), the wait is 4 s instead, so a slow scan cannot
   drop a live hint. Found by a unit test that had the screen read `working`
   when the hint arrived. Reason: the mod cannot see a permission prompt
   end (see (d)), so its close is best-effort, and a late close must not pin the
   pane blocked while the user is already running the tool.
4. **Reason precedence:** while a live hint exists its kind is the reason
   shown, even if the screen's rule says `other` or the other kind.
5. **Blocked spell:** the existing `blocked_spell` machinery runs on the
   effective state, so `blocked_since`, notifications and the agent panel
   treat a hint-driven block like a screen-driven one.

Age-out: a hint lives `ttl_ms` (default 15 s) from its last report, and the mod
**heartbeats**: while a dialog is open it re-reports the same hint every 5 s
(`$.clock.every`), so a question left open for ten minutes stays blocked, and a
mod that died (Claude SIGKILLed, mod unloaded, `/reload-plugins` mid-dialog)
stops refreshing and the hint ages out in at most 15 s. The spike's 10 minute
TTL is replaced by this pair; the TTL is deliberately three heartbeats so one
lost report does not drop a live dialog. Expiry is a deadline in server state,
checked by the event loop (a `hint_deadline` in the wake-up list and an
`expire_hints` in both ticks, `App` and the headless server), never in render.
When a pane's process exits its terminal state is dropped with its hint.

### A ceiling on a held hint

A heartbeat keeps a hint alive, so a close that was missed while the mod is
still alive (an event that never fired, a map entry that never emptied) would
hold a pane blocked for as long as Claude runs. Two ways to bound it were
weighed: a herdr-side rule (the screen clears a question hint after about 10 s
of continuous `working`) or a mod-side cap. **The mod stops heartbeating an id
after 30 minutes open, drops it from its map, sends the clear, and logs one
failure line; the 15 s TTL would end it anyway if the clear were lost.**

Reason: it keeps herdr's merge rule simple and true ("a question hint is held
by its close, whatever the screen shows"), where the herdr-side rule would put
a screen heuristic back into the one case the mod exists to make exact, and a
screen that shows `working` behind a real, long-open dialog is the case the
spike saw. The cost is that a question genuinely open for more than 30 minutes
reverts to screen detection, which reads such a dialog correctly in the normal
case, so the user loses nothing they have today. The cap is one constant in the
mod (`MAX_OPEN_MS`).

### (c) The `isOpen` flag

The mod keeps a single object `open: Map<tool_use_id, kind>` and a serialised
report queue. `close(id)` does nothing when the id is not open, so
`turn.complete` and `session.end`, which fire for every turn of every session,
spawn no process unless something is open. The heartbeat timer exists only while
the map is non-empty. Reports go through one queue so a `close` cannot overtake
the `open` it follows (the spike's `seq` is kept, and herdr drops a lower `seq`
anyway). Several open dialogs (parallel tool calls asking at once) keep the pane
blocked until the map is empty; the reported `id` is the oldest open one.

### (d) Permission prompts as the second reason

`tool.check` is the place: the hook does `const decided = await next(e)` and, if
`decided.decision === 'ask'`, a prompt is about to show for `e.tool_use_id`. The
hook records it in the open map as `permission` and reports a `permission` hint
**before returning** the decision (the prompt shows right after). Every agent
calls this, subagents too: a subagent's tool call that needs permission shows
the prompt in the parent's pane, so (unlike a question, which subagents cannot
ask in this build) the `agentId` guard does not apply to permission.

How the prompt's end is seen was measured, not guessed (milestone-2 spike,
Claude Code 2.1.288, Haiku, a mod that logs every candidate event with
timestamps; Bash command `rm -f <outside-cwd file> && sleep 8 && echo`, which
`tool.check` resolves to `ask`). Times are relative to the key press that
answers the prompt:

| Event | Allow (Enter) | Deny ("3") | Esc |
|---|---|---|---|
| `tool.check` | `{decision: "ask"}` 1.8 s before the key | same | same |
| `classic.PermissionRequest` | fires 10-20 ms after `tool.check`, when the dialog shows (no tool id) | same | same |
| `tool.call` `next.signal` abort | never | **+8 ms** | **+63 ms** |
| `tool.call` `await next(e)` settles | at the tool's end, **+8.2 s** | **+9 ms** (`isError`) | **+63 ms** |
| `classic.PermissionDenied` | never | **never fires** | never |
| `classic.PostToolUse` | at the tool's end, +8.1 s | never | never |
| `ui.render` `ToolProgress` | +3.0 s (`background_hint`, only if the tool runs 3 s) | never | never |
| `ui.render` `Spinner` | mode stays `tool-use`: no change | no change | no change |
| `ui.render` `ToolUse` | never fired in this build | | |

Findings: (1) deny and Esc are **exact**: `tool.call` aborts and settles in under
100 ms, so the same close the question uses works. (2) **Allow has no signal at
the moment of the answer**: nothing a mod can see changes until the tool ends,
so a long-running allowed tool would hold a permission hint for its whole run.
(3) `classic.PermissionDenied` never fires for a user's denial, and the
`Spinner`/`ToolUse` renders carry nothing, so neither is used. (4) `tool.check`
has `tool`, `input` and `tool_use_id`, and `decided` is `{decision, reason?}`.
(5) A mod's hooks module does **not load until the workspace is trusted**
("hooks modules not loaded until workspace trust is accepted" in the debug
log): in an untrusted directory no hint is ever reported and the screen
detects as today.

So the mod closes a permission hint on `tool.call` settling or aborting
(exact for deny, Esc, interrupts and any tool that ends soon), on
`turn.complete` and `session.end`; and for **allow of a long tool** herdr's own
rule does the work: the screen may clear a `permission` hint after 0.5 s of
not looking blocked (b.3). That is why the asymmetry in (b) exists, and the
measurements confirm it is needed rather than a precaution. A permission hint
is therefore wrong for at most about 0.5 s after an allow, never longer than
15 s without a heartbeat, and exact in the cases the mod can see. The
raise-only fallback is not needed.

**Decision after the proof run: the mod does not report permission prompts.**
The 6.2 numbers (issue 157): with a real Claude, the screen alone gave the same
reason and was as fast in every scenario (question answer 0.39 s, Esc 0.38 s;
permission allow 0.38 s, deny 0.81 s, Esc 0.64 s to leave `blocked`), so a
permission hint has no proven gain, and it has a measured cost: after an allow
the pane stayed blocked about 1.12 s instead of 0.38 s, every time (1.83 s with
the first 1.5 s grace), because a mod cannot see the answer to a permission
prompt. So the mod has no `tool.check` hook. The herdr side is unchanged:
`kind: permission` stays a valid hint, with the 0.5 s screen-clear rule, ready
for a mod or another source to use. **Re-enable with one hook if a missed or
late permission prompt is ever seen live:** `tool.check`, `await next(e)`, and
on `decision: 'ask'` `openDialog($, e.tool_use_id, 'permission')`; the close on
settle or abort in `tool.call` already covers it.

### (e) The install path

`herdr integration install claude` installs the mod, next to the settings hooks
it already writes. Reasons: the mod is part of herdr (it speaks herdr's API, so
it is versioned with `HERDR_INTEGRATION_VERSION` and shipped in the binary like
`src/integration/assets/claude/`), and `herdr integration install` is already
how a Mac gets herdr's Claude wiring and how a handoff upgrades it. A second
path through the macos-setup plugin manifest would carry a second copy of files
that must change in lockstep with the binary.

Mechanics: the command writes the plugin files to
`~/.config/herdr/claude-mod/` (plugin dir `herdr-attention/` plus a one-plugin
local marketplace manifest `herdr-local`), then runs
`claude plugin marketplace add <dir>` and
`claude plugin install herdr-attention@herdr-local --scope user`. Both were
run against a scratch config and are idempotent (a second run exits 0 with
"already ..."), and the plugin "loads in place" from the folder. The marketplace
is recorded in Claude's user settings (`extraKnownMarketplaces`). `uninstall` reverses both and removes the directory. The fleet manifest
(macos-setup) gets one line that runs `herdr integration install claude`; it
does not list `herdr-attention@...` itself.

Already-running Claude sessions do not have the mod: Claude loads a new or
updated plugin at its next start, or on `/reload-plugins`. The install output
says so ("running Claude sessions pick it up on /reload-plugins or restart"),
and nothing breaks meanwhile: those panes keep today's screen detection. Herdr
never requires the mod to be present. After a herdr upgrade, installing again rewrites the mod in
place (Claude loads the plugin from the folder, so no `plugin update` is
needed); running sessions keep the old mod until reloaded, which is safe because
the hint API is additive. The mod is **not** covered by `integration status`:
the Claude integration version is already ahead of the last release, and the
rule is to bump it once per release, so a second bump for the mod is not
allowed. A follow-up could give the mod its own marker; until then the dogfood
runs the install explicitly.

### (f) The Claude Code version gate and logging

Mods need Claude Code 2.1.287 or later. `install` runs `claude --version` and,
below the floor or with no `claude` on `PATH`, installs the settings hooks as
today, skips the mod, and says why in one line; this is a skip, not an error
(exit 0), because screen detection still works. The mod itself does not
version-check (an older Claude never loads it). The mod logs **only failures**:
a `$.ui.log` line when the helper exits non-zero or throws, at most one per
minute so a dead herdr cannot flood the transcript; no line on success, on
close, or on heartbeat (the spike's per-event lines were for debugging and
showed in the transcript).

### How the mod talks to herdr

Not through the CLI. The spike ran `herdr-beta pane report-metadata`, which
ties the mod to a binary name (`herdr` is the stale stable here and
`herdr-beta` the real one) and makes `HERDR_MOD_BIN` a fleet setting. The panes
already carry `HERDR_SOCKET_PATH` and `HERDR_PANE_ID`, and the existing Claude
hook script (`herdr-agent-state.sh`) already writes JSON to that socket. The mod
ships a sibling helper, `herdr-hint.sh` (a few lines of `sh` plus `python3`, as
the hook script does), run through `$.process.run([sh, helper, ...])`, which
sends one `pane.report_hint` request over the socket. It exits 0 and does
nothing when `HERDR_SOCKET_PATH` or `HERDR_PANE_ID` is unset, so the mod is inert
in a Claude that is not in a herdr pane.

## Risks

- **Stale hint.** A missed close pins a pane blocked: bounded by the TTL (15 s
  after the last heartbeat), by process exit, and, for permission, by the
  screen. Tests cover each bound.
- **Hook time budget.** The reporter awaits `$.process.run`, which does not
  count against the hook's 10 s, and a slow herdr never blocks the dialog: the
  open report runs before `next(e)` but a failure only logs.
- **Mod disabled.** `--safe-mode` and `disableAllHooks` turn it off; herdr falls
  back to the screen, which is today's behaviour.
- **Two sources.** A second source's hint on one terminal is ignored while the
  first is live. There is one source today (`herdr:claude-mod`); the rule keeps
  a future second one from fighting it.

## Verification

Unit tests for the hint state (set, replace by `seq`, clear, expiry at the
deadline, dropped on agent change) and the merge (question holds against an idle
screen; permission cleared 0.5 s after the screen stops showing it; no hint means
today's result), a server-tick test for expiry, schema and CLI tests. The mod's
own logic (open map, queue, heartbeat) gets `claude plugin test` tests. Milestone
2's throwaway proof uses a real Claude (Haiku) for question and permission
crossed with answer, Esc and kill, plus a pane without the mod, with the sampled
states posted on the issue.
