# Implement herdr fork issue #53 — move a pane to a different or new space

## Where you are

Repo `/Users/ac/_sync/dev/herdr` (personal fork of upstream herdr), `master` at
`d94ceb3a`, clean and green. Read `CLAUDE.md` first — it is layered and its rules
override defaults.

**Three remotes, don't confuse them:** `origin` = GitHub colangelo/herdr (the
fork), `upstream` = GitHub ogulcancelik/herdr (redirects to herdrdev), `internal`
= Gitea AC-forks/herdr. **The dev issue backlog is on Gitea, not GitHub.** Never
write to upstream — no issues, PRs, comments.

## The task

Implement OpenSpec change **`pane-move-to-space`** (`openspec/changes/pane-move-to-space/`,
20 tasks, `openspec validate --strict` clean). Issue:
<https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/53>

Read `proposal.md`, `design.md`, `specs/pane-move-controls/spec.md`, then work
`tasks.md` in order, checking boxes as you land each group. **The plan is
already decided — implement it, don't redesign it.** If you find a genuine
problem with the design, say so in a sentence and keep going under a stated
assumption rather than silently diverging.

### What it is, in one paragraph

`prefix+m` opens a picker that today lists only the other tabs of the *current*
space. Widen it to every space, plus a "new tab in space X" destination and a
"new space" destination. Upstream's `pane.move` already expresses all of this —
`PaneMoveDestination::{Tab, NewTab { workspace_id }, NewWorkspace}` in
`src/api/schema/panes.rs` — and the CLI already exposes it
(`herdr pane move <id> --new-workspace`). `NewWorkspace` is constructed **nowhere
in the TUI**, in this fork or upstream. So this is UI-only work over existing
server surface: no new state, no API field, no protocol bump.

### Code anchors (symbols, since line numbers drift)

- `pane_move_target_picker_for_state` — `src/app/input/navigate.rs`, builds the
  picker from `workspace.tabs` only. This is the function that widens.
- `open_pane_move_target_picker`, `dispatch_pane_move_with_feedback` — same file;
  every destination must keep routing through the dispatch helper so `pane.move`
  rejections surface as they do today.
- `PaneMoveTargetEntry`, `PaneMoveTargetPickerState` — `src/app/state.rs`, the
  entry shape that becomes a destination.
- `pane_move_target_inner_rect`, `render_pane_move_target_picker_overlay` —
  `src/ui/dialogs.rs`, the picker's geometry and rendering.
- `src/ui/keybind_help.rs` — every keybinding must appear here, **including while
  unbound** (the panel renders `unset`, which is how users discover it). Not
  optional: a shortcut absent from the help panel is treated as incomplete.

### Decisions already made (in design.md — don't relitigate)

- One picker, not a second keybinding. The list answers "where", not the chord.
- Grouped by space: own space first, then sidebar order. Headings are rendered
  but **not selectable** — selection steps over them.
- New space is created unnamed (server defaults), no prompt. Rename is one
  keystroke away afterwards.
- `focus: true` on every destination; the active space follows the pane.
- No filtering/search — deliberately deferred, spaces are few.

### One spec subtlety

The existing scenario *"No other tabs disables the action"* is **narrowed**, not
kept: with a new-space destination always offerable, the picker is suppressed
only when the pane has nowhere at all to go (single space, single tab, single
pane). The MODIFIED requirement in the delta spec carries both that narrowed
scenario and a new "a lone tab still opens the picker" one. Don't reintroduce the
old dead-end.

## House rules that will bite you

- **`just check` before committing.** Not `cargo test` — the recipe also runs the
  maintenance script suites. Use `cargo nextest run --locked --no-fail-fast` when
  iterating, because nextest cancels on first failure by default and hides the
  rest.
- **Commits:** lowercase conventional, no emojis, **no AI co-author lines**.
  Granular — separate implementation, docs, and planning artifacts. State the
  messages you chose and commit autonomously; don't stop for approval.
- Reference the Gitea issue by **full URL** in the commit body
  (`refs https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/53`). A bare
  `#53` is ambiguous across three hosts and is reserved for GitHub issues.
- **No `unwrap()` in production code.** `#[allow]` only with a comment saying why.
- Tests live next to the code; `AppState::test_new()` / `Workspace::test_new()`
  build state without PTYs.
- Docs for user-facing changes go in `docs/next/`, never root `README.md` or root
  `CHANGELOG.md`.
- Use a dedicated worktree (`wt`, not `git worktree`) if another agent holds the
  tree — a pre-write hook will tell you and name the holder. Never `--yes` a
  hook-approval prompt.

## Standing objective — this one matters here

The fork's direction is **making features easy to contribute back upstream**.
#53 is a strong PR candidate: `PaneMoveDestination::{NewTab, NewWorkspace}` is
upstream's own (`src/api/schema/panes.rs`, verified on `upstream/master`
`42789c8e`) and they ship no UI for it. **Keep it free of fork-specific styling**
(no editorial-sidebar colors, no fork-only config knobs). Task 5.3 checks this.

**Correction to an earlier framing — measure 5.3 against the right baseline.**
The widening diff alone does *not* lift cleanly, because the picker it widens is
itself fork-only. On `upstream/master`, `pane_move` appears only in the API and
CLI layers (`src/app/api.rs`, `src/app/api/panes.rs`, `src/cli/pane.rs`); it
appears **nowhere** in `src/app/input/navigate.rs`, `src/ui/dialogs.rs`, or
`src/app/state.rs`. So `pane_move_target_picker_for_state`,
`render_pane_move_target_picker_overlay`, `pane_move_target_inner_rect`,
`PaneMoveTargetEntry`, and `PaneMoveTargetPickerState` are all fork-only. An
upstream PR therefore carries the whole picker — base plus widening — as one
`feat:`, not a small delta on existing upstream UI. That is still a clean PR;
it is just a larger one, and 5.3 should read as "no fork opinion in the combined
picker" rather than "this diff applies to upstream as-is".

Same reason dissolves the `widgets.rs` tension below: `FOOTER_ROWS` and
`footer_split` are also fork-only (`src/ui/widgets.rs` exists upstream, those two
helpers do not). Using them adds no *new* upstream friction, since the picture
being ported is fork-only regardless — inline the two-row footer reservation at
lift time. Prefer the helpers, so `overlay-ui-kit` absorption stays a move.

## Also true

- `overlay-ui-kit` (`openspec/changes/overlay-ui-kit/`, 0/34) will later fold
  panel geometry into a shared kit. Express new rendering through
  `src/ui/widgets.rs` helpers — including `FOOTER_ROWS` / `footer_split`, which
  reserve the blank row above footer buttons — so that absorption stays a move,
  not a rewrite.
- GitHub token scopes are `gist, read:org, repo`. Pushing a commit that modifies
  `.github/workflows/` will be **rejected** until someone runs
  `gh auth refresh -h github.com -s workflow`. Ordinary pushes are fine.
- `rerere.autoupdate` is on globally; it wedges `rebase --continue` with "you
  have staged changes". Only relevant if you rebase.
- The docs translation heading-parity gate was removed on 2026-08-13
  (`405d313f`): the fork is English-only, so an English-only heading no longer
  fails `just check`. `release-docs-check` still enforces *file-set* parity, so a
  brand-new `.mdx` page would still need `ja/` and `zh-cn/` counterparts —
  adding headings to an existing page does not.

## Done means

`just check` green, tasks.md boxes checked, docs staged under `docs/next/`,
committed and pushed to **both** `origin` and `internal`, CI green on the pushed
master, and a resolution comment on the Gitea issue before closing it
(convention: done = closed, no terminal label).

## Afterwards — #54, only if asked

`todo-board` (`openspec/changes/todo-board/`, 25 tasks) — a session-wide board
over `todo.list`'s existing aggregate. **It has prerequisites: `pane-todos-ux`
(24/25) and `todo-editing-keyboard` (31/31) must be archived first**, since its
group 3 reuses behaviour those changes are still holding open. Don't start it
without confirming that, and don't archive those two without checking with the
user — `pane-todos-ux` has one live dogfood task outstanding.
