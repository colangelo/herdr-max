# Design

## Context

Three reported gaps, one shared root: the todo feature is hard to reach and its
link control does not scale. Investigation showed the storage layer is already
more capable than the UI exposes, so most of this change is UI, plus one
correctness fix.

Facts established by reading the code, not assumed:

- `PaneId` is allocated from a single global `AtomicU32` (`src/layout.rs`), so
  pane identity is unique across the whole session, not per workspace.
- `AppState::pane_todo_link_target` already searches every workspace to resolve a
  link, so following a cross-workspace link would already work.
- `src/persist/restore.rs` already anticipates cross-workspace links — its own
  comment says a target may be "one in another workspace entirely" — and defers
  every link to a restore-wide id map resolved in a post-pass.
- `AppState::pane_link_candidates` is the only thing scoped to one workspace.
- The navigator's pane rows (`src/app/actions.rs`) already label panes far better
  than the link code: `manual_label → agent_name → effective_agent_label →
  launch_label(launch_argv) → "pane N"`, with `meta` of `agent · status` or
  `"shell"`, and a precomputed lowercase `search_text`.

## Goals / Non-Goals

**Goals**

- The todo affordance is in the same place on every pane, including empty ones.
- A todo can be created from the panel, by mouse and by key.
- A todo can link to any pane in the session, found by search rather than by
  cycling, and named usefully when it is a plain shell.
- Cross-workspace links persist rather than silently vanishing.

**Non-Goals**

- No change to the todo store, snapshot format, socket API, protocol, or CLI.
- No general modal stack. The one nested case here is handled by an explicit
  return path, matching how the edit modal already returns to the panel.
- Not building a second list widget. The navigator is reused rather than
  duplicated.

## Decisions

### Indicator: always drawn, three tones

`pane_todo_indicator_label` currently returns `None` when a pane has no todos.
It will always return a glyph. This creates a collision the naive version misses:
"no todos" and "all done" both render a bare glyph and would become
indistinguishable, silently destroying an existing signal.

Resolved by tone rather than by glyph, which keeps the reserved width identical
across states:

| State | Content | Tone |
| --- | --- | --- |
| Outstanding todos | glyph + count | highest outstanding priority colour |
| All done | glyph | normal dim |
| No todos | glyph | dimmest (`overlay0`) |

`ui.show_pane_todo_indicator = false` continues to suppress every state.

Two existing guards stay exactly as they are, and the spec was corrected to match
them after reading the code:

- **No top border, no indicator.** A single-pane tab or `ui.pane_borders = false`
  draws no chrome to hang it on; the bindable action is the path there. "Every
  pane" in the proposal means every pane that draws a top border.
- **The indicator wins a squeeze.** It is laid out first and the title takes what
  is left, dropping itself when that is too narrow — not the other way round. The
  indicator is omitted only when the pane cannot carry the glyph plus its two
  border corners (`rect.width < width + 2`).

**Alternative rejected:** showing the empty-state glyph only on the focused pane.
Cheaper on title width, but it makes the affordance appear and disappear as focus
moves, which is worse for discoverability than the width it saves. Explicitly
chosen against.

**Cost accepted:** every pane's title loses the reserved columns permanently.
This was weighed and accepted; uniform placement was the point.

### Panel: add action

`a` is free in the panel keymap (`j/k`, arrows, `Enter`, `Space`, `g`, `d`, `c`,
`Esc`, `q`) and binds to the existing `AppState::open_new_pane_todo`. No new
return-path machinery is needed: `close_pane_todo_edit_and_return` already
returns to `Mode::PaneTodos` when the panel is open.

A footer button is added because this is a mouse-first TUI. The empty state must
also render the footer — today it deliberately renders none, which combined with
an always-on indicator would make clicking a quiet pane a dead end.

### Link picking: the navigator in a selection mode

The navigator already enumerates every pane in every workspace with good labels,
search, a state filter, and tree expansion. Reusing it gives the picker its whole
feature set for a small delta and keeps one interaction language, per the "UI
patterns should be reused" rule.

The navigator gains a purpose. In selection mode:

- pane rows resolve the selection and return, instead of focusing;
- workspace and tab rows are context only — activating one expands or collapses
  it and never resolves the selection;
- a synthetic entry that clears the link is offered;
- the todo's own pane is not offered;
- dismissal returns to the edit modal leaving the staged link untouched.

`pane_todo_edit` lives on `AppState` independently of `mode`, so it survives
while the navigator is open. The return path mirrors
`close_pane_todo_edit_and_return`.

`cycle_pane_todo_edit_link` and `pane_link_candidates` are deleted rather than
left alongside. Two mechanisms for one job is how they drift.

**Alternatives rejected:** a dedicated flat picker reusing only the row builder
(cleaner isolation, but a second list renderer to keep visually in step); and
extracting a shared searchable-list overlay used by both (best structure, but it
refactors a core UI/input surface, which the repo classifies as release-risk and
would require navigator characterization tests first). Both remain open if the
navigator's second purpose turns out to strain it.

### The link label is captured on the server, not by the picker

Easy to get wrong, because the picker is where the good label is visible. It is
not where the label is stored. The TUI sends only `link_pane_id`; the server's
`resolve_link` (`src/app/api/todos.rs`) builds the stored label from
`manual_label → effective_agent_label → raw`. A shell with neither is therefore
persisted as its raw public pane id, `w1:p3`.

The navigator's chain has a fourth link the server's lacks:
`launch_label(launch_argv)`. So the picker showing `npm run dev` while the saved
chip reads `w1:p3` is not a hypothetical — it is what shipping the picker alone
would produce, reproducing the very complaint this change answers.

The fallback is therefore added to `resolve_link`, and `launch_label` is shared
out of `src/app/actions.rs` rather than duplicated.

**Why server-side rather than sending a label from the picker:** the captured
label is a shared runtime fact persisted on the todo and read back by every
client, not TUI presentation state. Letting one client inject it is exactly the
coupling the runtime/client boundary guardrail forbids, and it would need a new
API field. Fixing the chain instead keeps the wire unchanged and fixes CLI
callers of `herdr todo add --link` for free.

### The cross-workspace save fix

`App::public_pane_id(ws_idx, pane_id)` looks the pane up *inside*
`workspaces[ws_idx]`, and `save_pane_todo_edit_via_api` passes
`self.state.active`. A target in another workspace therefore resolves to `None`
and the link is dropped on save with no error.

A session-wide lookup is added — find the workspace containing the pane, the same
search `pane_todo_link_target` already performs, then build the public id from
that workspace — and the save path uses it.

This must land together with the widened candidate set. Widening alone produces
links that appear to be set, then vanish on save: a worse failure than not
offering them.

## Risks / Trade-offs

- **The navigator gains a second purpose.** Mitigated by keeping selection
  behaviour to a purpose flag consulted at activation, not threaded through
  rendering. If it starts leaking, the extraction alternative above is the exit.
- **Selection mode shares navigator transient state** (query, expansion,
  selection). Acceptable: that state is already transient per open.
- **Title width regression on every pane.** Accepted deliberately.
- **`ctrl+l` changes meaning** from cycle to open-picker. Muscle-memory break for
  anyone who learned the cycle; the cycle was unusable past a few panes.

## Migration

None. No persisted shape changes, and existing links keep resolving — the stored
`TodoLink` is unchanged and already carries a session-unique pane id.
