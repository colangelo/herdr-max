# Implementing overlay-ui-kit

`proposal.md`, `design.md`, `specs/` and `tasks.md` carry the what and the why,
and the task order is deliberate: the snapshot harness is the safety net and
lands before any geometry moves. Implement it, don't redesign it. This file is
only the operational things that are not in those, plus corrections to anchors
that have drifted since the plan was written.

Tracking issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/65

## Every anchor in tasks.md has moved — re-verified 2026-08-16 at master d81beb85

The plan is 19 days old and the tree moved under it. Verified by grep, not
recalled:

| tasks.md says | actually |
|---|---|
| `notification_center_rect` `mouse.rs:1471` | `src/app/input/mouse.rs:1524` |
| `pane_todo_panel_rect` `mouse.rs:1589` | `src/app/input/mouse.rs:1635` |
| `pane_todo_panel_button_rects` `todo_panel.rs:45-121` | `src/ui/todo_panel.rs:39-` |
| `notification_center_button_rects` `notification_center.rs:41-100` | `src/ui/notification_center.rs:34-` |
| `MenuListState` `state.rs:1132` | `src/app/state.rs:1087` |
| `SelectionListState` `state.rs:1159` | `src/app/state.rs:1114` |
| `wants_ascii_input` `state.rs:843` | `src/app/state.rs:845` |
| `honors_key_repeat` `state.rs:881` | **does not exist** — see below |

**Task 6.4 is wrong about the second allowlist.** There is no
`honors_key_repeat` on `Mode`. The key-repeat gate is
`App::terminal_input_context()` at `src/app/mod.rs:1855`, an if-chain over
`Mode` returning `Option<TerminalInputContext>`, reshaped by the
`app-scroll-key-repeat` change after this plan was written. It lives on `App`,
not `AppState`, because the popup arm needs `popup_pane`. Deriving it from the
overlay variant is still the right idea; the mechanical instruction in 6.4 is
not. Treat that task as "derive `wants_ascii_input` from the variant, and
account for `terminal_input_context` separately" and say so in the commit.

## Leads that save a research pass

**The `TextField` prerequisite is already in the tree.** `tasks.md` names
`todo-editing-keyboard` group 1 as a prerequisite; that change is archived and
`TextField` is at `src/ui/text_field.rs:56`. Group 5 can adopt it immediately.

**There are three `row_text` test helpers, not one.** Task 1.1 names
`src/ui/todo_panel.rs:420`; `src/ui/panes.rs:1892` and `src/ui/sidebar.rs:2196`
have their own, with a different signature (`row: u16, width: u16` rather than a
`Rect`). Promote the todo-panel one as the task says, and fold the other two in
while you are there — leaving three is how this class of duplication started.

**Group 6 needs the roundtable before code, not after.** `AGENTS.md` classes it
as refactor-risk (two or more core surfaces, plus UI/input state projection).
The identity invariants it wants are `AppState::assert_invariants_for_test` with
`AppState::test_with_adversarial_identity_state`. Note the trap that bit the
`respawn-pane` change: `confirm_close_pane` is deliberately **not** asserted
there — it stays honest through the per-pane cleanup in
`AppState::forget_pane_todo_ui` (`src/app/state.rs`). Do not add a lone
assertion for a field whose contract is a cleanup hook.

## Archiving traps, learned the hard way today

**Archiving syncs specs, and that can break a change you are not touching.**
Archiving `todo-editing-keyboard` added scenarios to two `pane-todos`
requirements that the still-open `pane-todos-ux` change also MODIFIES. A
MODIFIED block replaces the whole requirement, so `openspec validate --strict`
then refused it for silently dropping six scenarios. Before archiving anything,
check which capabilities the *remaining* open changes touch, not just the ones
you are archiving together. The fix is to copy the new scenarios into the open
change's MODIFIED block verbatim, in spec order, keeping that change's own
versions of the scenarios it actually modifies.

**`openspec archive` writes a TBD purpose for a new capability.** It emits
`## Purpose\nTBD - created by archiving change <name>. Update Purpose after
archive.` Fill it in before committing. Eight specs in `openspec/specs/` still
carry that placeholder from older archives if you want to sweep them.

## Dogfooding and the beta channel

The loop is `just check` → commit → push → `just beta <ref> [codename]` →
`just brew-upgrade herdr-beta`, which live-hands-off the running server without
killing panes. `just beta` takes a **branch**, so you never merge to master to
test. The optional third argument pins the build codename (added today); it must
be a name from the `NAMES` pool in `.github/workflows/beta.yml`, and the run
number still increments so `brew upgrade` ordering is untouched. The user asked
for `pirlo`; keep using it unless told otherwise.

Pushing anything under `.github/workflows/` needs the `workflow` scope on the
`colangelo` gh token. It has it now.

## House facts that cost time if you learn them the slow way

- **Issues live on Gitea `AC-forks/herdr`, not GitHub.** Reference them in
  commits by full URL — a bare `#N` is ambiguous across three remotes.
- **`tests/cli` never runs on macOS** (`#![cfg(not(target_os = "macos"))]` at
  the top of `tests/cli.rs`, tracked as Gitea #30). If you add an integration
  test there, temporarily blank that line to run it locally, then restore it.
- **`just check` includes a Windows clippy leg**, so anything using a
  `#[cfg(unix)]` helper needs gating or a `cfg(not(unix))` arm. It fails late,
  after the whole test suite.
- **`reload_aborts_an_in_flight_command_task_and_its_descendants` is flaky
  under load** (Gitea #57). It has failed a full `just check` here while a beta
  was compiling in parallel, and passes 6/6 in isolation. Re-run before
  believing it.

## After this: the queue

1. **`pane-todos-ux`** is 24/25. The only open task is 6.4, a mouse dogfood the
   agent cannot do: click an empty pane's todo indicator, add a todo from the
   panel, link it to a pane in another workspace. Ask the user to run it, then
   archive the change.
2. **`overlay-ui-kit`** — this file.
3. **`todo-board`** (Gitea #54, 25 tasks). Deliberately after the kit, so the
   board is built on it rather than becoming one more bespoke overlay to
   migrate. Its group 2 ("the board surface") is where the kit pays off.
