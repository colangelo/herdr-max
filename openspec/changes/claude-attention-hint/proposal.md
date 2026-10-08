## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/137 part C (ac: "find
a stable fix for this"). Parts A and B made an open Claude AskUserQuestion
dialog read `blocked` (reason `question`) from the screen. The screen can
still miss a dialog: a hidden prompt box, a narrow pane, a new Claude layout.
ac's watcher presses Esc on a question open for 3 minutes, so a missed
question is a stuck session. A Claude hook can say "a dialog is on screen"
without reading it.

Status: **proposed, not approved.** Code waits for ac's approval.

## What upstream learned (why Claude hook state was removed)

Upstream removed hook-driven Claude state in `295b09ca` because the hooks do
not cover the lifecycle:
- subagents' `PermissionRequest` marked the parent pane blocked (#58);
- no hook fires after a rejected or Esc'd prompt, so blocked stuck (#349);
- a late `PostToolUse` re-pinned working after the turn ended (#198, the source
  of `HOOK_REMOVALS`);
- hook and screen fought: ~71% false blocked (#408).
The deeper cause: a hook either owns a pane's state or only reports its
session; there is no "evidence the screen can clear".

## What Changes

- **One hook, `PermissionRequest`** (Claude fires it "when the user is shown a
  permission dialog"; in every real attended AskUserQuestion on record it
  fired in the same second, with `tool_name = AskUserQuestion`). It reports an
  **attention hint**, `{reason: question | permission}`, through the existing
  session report path. Subagent calls (`agent_id` set) are ignored, as the
  hook asset already does. `PreToolUse`, `PostToolUse` and `SubagentStop`
  stay removed.
- **The hint is evidence, not authority.** While it is live, a *weak* screen
  answer (the OSC-title idle rule or the default idle fallback) reads
  `blocked` with the hint's reason. Any *strong* screen rule wins.
- **It clears** on the same session's next `UserPromptSubmit`, `Stop` or
  `SessionStart` (already installed), on process exit or session change, when
  a strong screen rule (body working, screen blocked, or the idle prompt box
  that returns after Esc) holds for about 1 second, or after 30 minutes.
  "No dialog visible" alone does not clear it: that is the case the hint is
  for.
- Not consuming gestore-lab's `events.jsonl`: it only exists in controlled
  repos while a controller runs, mixes hook denials with real dialogs, and has
  no subagent or cancel information.

## Capabilities

### New Capabilities

- `claude-attention-hint`: a Claude dialog hook adds clearable `blocked`
  evidence.

## Impact

- `src/integration/claude_settings.rs` + assets: one canonical
  `PermissionRequest` entry; integration version bump.
- `src/integration/assets/claude/herdr-agent-state.sh`: an `attention` mode.
- API: one neutral optional field on the session report
  (`attention`), and `TerminalState.attention_hint`.
- `src/terminal/state.rs`: weak-screen override and clear rules.
- Docs: integrations.mdx (the "state comes from the screen" note gains the
  hint exception).

## Open risk to measure first

Whether `PermissionRequest` fires for AskUserQuestion under
`--allow-dangerously-skip-permissions` / bypass modes (ac runs them). Task 1
measures it in a throwaway session before any code. If it does not fire, the
fallback is `PreToolUse` matcher `AskUserQuestion`, accepting a false hint
when another hook denies the call (cleared by the next prompt or Stop).
