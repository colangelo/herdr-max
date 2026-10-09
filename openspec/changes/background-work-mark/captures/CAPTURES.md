# Live captures of Claude Code background work (issue 172 task 0.1, #78, #137)

Captured 2026-10-08 in a throwaway herdr session. Binary: `herdr-beta 0.8.2-ac-beta.146` (the **master**
manifest, which still has `background_shell_working`). Agent: Claude Code 2.1.295, `claude-haiku-5-5`,
manual mode. Source: `agent read --source detection`. `.txt` files here; the `.ansi` and `.json`
(`agent explain`) copies stay in the untracked `.local/captures/`.

| file | what was running | footer / screen shows | herdr state (rule) |
|---|---|---|---|
| `a-one-shell` | 1 `sleep` shell | footer `⏸ manual mode on · 1 shell · ← 1 agent`; title `✳ Background sleep 300 command` | working (`background_shell_working`) |
| `b-two-shells` | 2 shells | `… · 2 shells · ← 1 agent` | working (same rule) |
| `c-one-subagent` | 1 background subagent whose own `sleep` counts as a shell | `… · 3 shells · ← 1 agent`, then panel rows `⏺ main` / `◯ general-purpose  Sleep 120 seconds and report   11s · ↓ 79.3k tokens` | working, but only through the shell rule |
| `d-two-subagents` | 2 subagents + the shells | footer becomes `· 3 shells, 1 monitor · ← 1 agent` (comma list, new kind `monitor`) | **idle** (`live_prompt_box`): MISSED |
| `i-two-subagents-no-shells` | 2 subagents, no shells | `⏸ manual mode on · 2 monitors · ← 1 agent` + two `◯ general-purpose …` rows | **idle**: MISSED |
| `e0-workflow-prompt` | workflow start dialog | `Run a dynamic workflow?` style dialog: "Yes, run it / View raw script / No", `Esc to cancel · Tab to amend` | blocked (`dynamic_workflow_prompt`) |
| `e1-workflow-running` | a workflow (`two-hellos`) | footer `· 2 shells · /tasks to see subagents · ← 1 agent` and row `◯ two-hellos  ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰  2/2 · 3s · ↓ 161.2k tokens` | working (`osc_title_working`) |
| `e2-workflow-done` | workflow finished, shells remain | summary line `✻ Sautéed for 49s · done 10:38 PM · 2 shells still running` | working (shell rule) |
| `g-idle-with-shells` | idle prompt, shells still running | prompt box empty, footer `· 2 shells · ← 1 agent` | working (shell rule): correct |
| `f-askuserquestion` | AskUserQuestion dialog | `☐ Colour`, options, `Enter to select · ↑/↓ to navigate · Esc to cancel` | blocked (`ask_user_question_dialog`): correct here |
| `h-after-esc` | dialog cancelled, 2 shells | `· 2 shells · ← 1 agent` | working |

## Findings

1. **The footer is the count.** Claude prints `· N shell(s)` and, with other kinds present,
   a comma list: `3 shells, 1 monitor`, `2 monitors`. Kinds seen: `shell`, `monitor`. A background
   subagent shows as a `monitor` in the footer plus a `◯ <type>  <title>  <age> · ↓ <tokens>` row
   below it; a workflow shows as a `◯ <name>  ▰▰▰…  done/total` row.
2. **`background_shell_working` (master) misses the comma list.** Its regex
   `·\s+[1-9]\d*\s+shells?\s*(?:·|$)` needs `shells` to end at `·` or the line end, so
   `3 shells, 1 monitor` and `2 monitors` fall through to `live_prompt_box` = **idle**. A pane with
   only subagents running reads idle. This is the miss behind #78.
3. **The port dropped the shell rule.** `sync/merge` `src/detect/manifests/claude.toml` has
   no `background_shell_working` at all (only `background_agents_working` and
   `background_mcp_task_working`), so on the 0.9.3 base even case `a`/`b`/`g` read idle until the rule is
   restored. Verify at cutover.
4. **Invariant lines a rule can match** (region: bottom lines under the prompt box): the footer line
   starting `⏸`/`⏵` containing `·`-separated segments; each segment `N <kind>[s]` with kinds
   `shell|monitor|…`. One rule with an `N <kind>` segment list covers all cases and gives the braille count
   as the sum of the segment numbers (`3 shells, 1 monitor` = 4).
5. **Count caveat.** A subagent's own shell is counted too (`3 shells` with one subagent), so the footer
   sum is "things Claude tracks", not "things the user launched". For the braille mark this is the
   honest number to show.
6. #137: AskUserQuestion read `blocked` here (rule `ask_user_question_dialog`); the "reads idle" case was
   not reproduced with a visible prompt box.
