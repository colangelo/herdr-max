# herdr — colangelo fork

<p align="center">
  <img src="assets/logo.png" alt="herdr" width="100" />
</p>

<p align="center">
  <a href="#install">install</a> · <a href="#what-this-fork-adds">what this fork adds</a> · <a href="#release-channels">channels</a> · <a href="./CHANGELOG.md">changelog</a> · <a href="https://herdr.dev/docs/">upstream docs</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-666666?labelColor=333333" alt="Apache 2.0 license" /></a>
  <a href="https://github.com/colangelo/herdr-max/releases/latest"><img src="https://img.shields.io/github/v/release/colangelo/herdr-max?include_prereleases&label=release&labelColor=333333&color=666666" alt="latest fork release" /></a>
  <a href="https://github.com/colangelo/homebrew-tap"><img src="https://img.shields.io/badge/brew-colangelo%2Ftap-666666?labelColor=333333" alt="Homebrew tap" /></a>
  <a href="https://github.com/herdrdev/herdr"><img src="https://img.shields.io/badge/fork_of-herdrdev%2Fherdr-666666?labelColor=333333&logo=github" alt="fork of herdrdev/herdr" /></a>
</p>

---

**agent multiplexer that lives in your terminal** — every agent at a glance (blocked, working, done), real terminal views, detach and reattach from anywhere, a pure socket api agents can drive themselves, keyboard and mouse both first-class, one rust binary.

This is a **hard-tracking fork** of [herdrdev/herdr](https://github.com/herdrdev/herdr). It rebases onto upstream `master` regularly and carries ~190 commits of extra work on top: per-pane todos, a notification center, a heavily configurable sidebar, layout and pane-styling controls, copy-mode ergonomics, and its own release, update, and Homebrew pipeline. Everything upstream ships is here; the list below is what upstream does *not* have.

## what this fork adds

### pane todos

A durable list of next steps attached to each pane — the fork's largest addition.

- Agents write their own: `herdr todo add "rerun the flaky test"` inside a pane defaults to that pane, no ID needed. Todos carry a priority (high/normal/low), a done flag, and an optional link to another pane. Capped at 50 per pane / 500 chars each, persisted in the session snapshot, and they survive server restarts and `herdr update --handoff`.
- Every pane that draws a top border shows a `▾` indicator at its right end: outstanding count colored by the highest priority, a bare `▾` when everything is done, dimmed further when the pane has none — so the control is always in the same place and an empty pane is still clickable.
- `prefix+ctrl+t` (or a click) opens a panel anchored to that pane: `j`/`k` to move, `a` to add, `Enter` to edit, `Space` to toggle done, `g` (or the footer's `go`, or the chip) to jump to a linked pane, `d` to remove, `c` to clear completed. In the edit view, `ctrl+g` saves and follows in one keystroke.
- The edit view cycles priority with `Tab`, toggles done with `ctrl+t`, saves with `ctrl+s`, and picks a link with `ctrl+l` — the link picker is the session navigator in selection mode, so any pane in any workspace is searchable, moves under `ctrl+j`/`ctrl+k` even while searching, and names a plain shell by the command it launched.
- Its text field is a real one: an insertion point, the readline chords (`ctrl+a`/`ctrl+e`, `ctrl+b`/`ctrl+f`, `alt+b`/`alt+f`, `ctrl+d`, `ctrl+k`/`ctrl+u`, `ctrl+w`, `ctrl+y`, undo on `ctrl+_`), and newlines — `Enter` adds a line, which is why saving moved to `ctrl+s`. The panel keeps one row per todo, showing the first line with a `⏎` marker.
- A live link reads `→ w2:pC · claude`: the public pane id first, because that is what you paste into `herdr pane` or a sibling agent's prompt, then the captured label. Resolved live, so it follows a pane that moves. A todo whose target pane is gone renders dimmed and inert with its label alone.
- Closing a pane with outstanding todos asks for confirmation, over the socket API as well as in the TUI.
- Exposed as `todo.list` / `todo.add` / `todo.update` / `todo.remove` / `todo.clear` plus a `todo.changed` event, and `herdr todo list [--all] [--json]`, so external status bars can read the same state.
- `ui.show_pane_todo_indicator`, `ui.pane_todo_color`, `keys.open_pane_todos`, `keys.add_pane_todo`.

### notification center

- The server keeps a bounded log (newest 100) of every in-app toast — needs-attention, finished, update, and `notification.show` injections — with a per-entry read flag.
- A `◆` indicator with an unread pill sits at the right edge of the tab bar; `prefix+ctrl+n` or a click opens a newest-first dropdown. Unread entries carry a kind-colored dot and a bold title; visited ones dim. Opening the panel does not mark anything read.
- `Enter` jumps to the notification's pane and marks it read, `r` marks all read, `c` clears the log.
- `ui.notification_center_position = "bottom-right"` floats the indicator in the frame corner with the dropdown above it.
- `notification.list` / `notification.mark_seen` / `notification.clear` socket methods, a `notification.posted` event, and `herdr notification list|clear [--json]`.

### sidebar

- **Jump labels** — `ui.show_workspace_numbers` / `ui.show_agent_numbers` render each entry's jump target, continuing past 9 with `a`..`z`. Colorable (`ui.workspace_number_color`, `ui.agent_number_color`) with optional leader glyphs (`ui.workspace_number_prefix`, `ui.agent_number_prefix`) that hint the chord, e.g. `₽⌥2`.
- **`a..z` indexed keybindings** — `focus_agent = ["prefix+alt+1..9", "prefix+alt+a..z"]` and friends. On macOS the ranges skip `e`, `i`, `n`, `u` (Option dead keys that never reach the app), and the label and the matcher derive from one filtered sequence so a label can never name a key that does nothing.
- **Attention sort** — `ui.workspace_sort = "priority"` live-sorts spaces by state (blocked, done, working, idle), worktree groups moving as one unit.
- **Bubble motion** — reordered rows hold their place for `ui.sort_motion_settle_ms`, then travel one slot per `ui.sort_motion_step_ms` instead of teleporting, so the list stops reshuffling under the cursor. `ui.sort_motion_easing = "bubble"` eases the cadence; `ui.sort_motion = "instant"` restores immediate re-sorting.
- **Editorial style** — `ui.sidebar_style = "editorial"` moves jump numbers to the right edge, renders thin uppercase section headers, and dims inactive meta lines. `[ui.state_colors]` overrides the working/idle/done/blocked/unknown glyph and text colors everywhere they appear.
- **Active-row emphasis** — `ui.sidebar_active_border` (`above`/`below`/`both`/`left`/`right`).
- **Hidden rows say so** — `ui.sidebar_overflow` (`both`/`rows`/`fog`/`off`): edge rows summarise what the spaces list and agent panel have scrolled out of view, and the rows next to the edge get a lighter background.
- **Follow focus** — the spaces and agents lists scroll just enough to keep the focused entry visible across any focus change or reorder; manual scrolling disengages, the next focus change re-engages.
- **Collapsed parity** — the collapsed rail highlights the active agent, uses real jump letters past the 9th, and honors the number colors and the left/right active border.
- **`ui.show_host`** — the server's short host name, right-aligned on the `SPACES` header, so you can tell which machine a session is on. Also on the API `ping` response.

### panes, layout, and copy mode

- **Layout presets** — `even_horizontal` / `even_vertical` / `tiled` via `layout.set_preset`, cycled with `next_layout` (`prefix+space`), plus `herdr pane layout --set <even-h|even-v|tiled>`.
- **Balance** — `balance_panes` (`prefix+=`), the `layout.balance` socket method, and `herdr pane balance`.
- **Pane to tab or space** — `break_pane` (`prefix+!`), `move_pane_to_tab` (`prefix+m`, destination picker covering tabs in any space, a new tab, or a new space), `move_pane_next_tab` / `move_pane_prev_tab` (`prefix+>` / `prefix+<`), all preserving the running terminal.
- **Clear scrollback** — `pane.clear` with tmux `clear-history` semantics, `herdr pane clear`, and the `keys.clear_scrollback` binding.
- **Styling** — `ui.pane_border_active_color` / `_inactive_color`, `ui.pane_border_active_style` (light/heavy/double), `ui.pane_title_active_color` / `_inactive_color`, `ui.pane_active_bg` / `ui.pane_inactive_bg` (tmux `window-active-style`), and `ui.dim_inactive_panes` for always-on dimming of unfocused panes.
- **Copy mode** — one-gesture pre-scrolled entry with `prefix+pageup`, `prefix+ctrl+u`, `prefix+ctrl+k` (tmux `copy-mode -u`); `ctrl+k` / `ctrl+j` line-wise viewport scroll (vim `ctrl+y`/`ctrl+e`); held escape-coded keys now repeat instead of firing once.
- **Toasts** — a `center` position floating over the pane area, `ui.toast.herdr.size` presets, and per-kind durations (`needs_attention_seconds`, `finished_seconds`, `update_seconds`).

### runtime and release plumbing

- **Handoff resurfaces working agents** — the `herdr update --handoff` manifest carries each pane's live agent state and the new server sweeps every pane in the background, so working and blocked panes show their real state within seconds instead of all restoring as idle until viewed.
- **Fork-scoped update checks** — the binary reads the fork's own `website/latest.json` / `preview.json`, so `herdr update` never pulls an upstream build over a fork install.
- **Its own release pipeline** — `-ac`-suffixed stable tags plus a rolling `-ac-beta` prerelease channel with human-readable build IDs (run number + a Juventus surname), both publishing four binaries and updating [colangelo/homebrew-tap](https://github.com/colangelo/homebrew-tap).
- **Live channel migration** — switching between stable and beta, or upgrading a brew install, goes through a live handoff instead of killing panes.
- **OpenSpec workflow** — every non-trivial change lands with a committed `openspec/changes/<name>/` proposal, design, spec deltas, and task list, mirrored to the Claude, Codex, and Pi skill directories.

Full detail for everything above is in [`CHANGELOG.md`](./CHANGELOG.md) and the staged [`docs/next/`](./docs/next/) docs.

## install

Homebrew (this is how the fork distributes):

```bash
brew install colangelo/tap/herdr
```

or grab a binary from [releases](https://github.com/colangelo/herdr-max/releases/latest) — `herdr-macos-aarch64`, `herdr-macos-x86_64`, `herdr-linux-x86_64`, `herdr-linux-aarch64`.

> Do **not** use `curl https://herdr.dev/install.sh` — that installs upstream and will overwrite a fork install.

Then start it where the work lives:

```bash
herdr
```

`ctrl+b q` detaches, `herdr` reattaches. Upstream's [quick start](https://herdr.dev/docs/quick-start/) applies unchanged.

## release channels

| channel | install | updates |
| --- | --- | --- |
| stable | `brew install colangelo/tap/herdr` | `-ac` tags, e.g. `v0.7.4-ac` |
| beta | `brew install colangelo/tap/herdr-beta` | rolling `-ac-beta` prerelease, rebuilt on demand from `master` |

`herdr update` checks the fork's manifests; `herdr update --handoff` upgrades a running server without dropping panes.

## docs

Base behavior is upstream's and documented at [herdr.dev/docs](https://herdr.dev/docs/): [concepts](https://herdr.dev/docs/concepts/) · [supported agents](https://herdr.dev/docs/agents/) · [keyboard](https://herdr.dev/docs/keyboard/) · [configuration](https://herdr.dev/docs/configuration/) · [session state](https://herdr.dev/docs/session-state/) · [remote](https://herdr.dev/docs/persistence-remote/) · [plugins](https://herdr.dev/docs/plugins/) · [socket api](https://herdr.dev/docs/socket-api/)

The fork publishes no website. Its documentation lives in the repo, under [`docs/next/website/src/content/docs/`](./docs/next/website/src/content/docs/) — the configuration, keyboard, CLI, and socket API pages there are regenerated to include the fork's keys, bindings, verbs, and methods.

## development

```bash
git clone https://github.com/colangelo/herdr-max
cd herdr
cargo build --release

just test        # unit tests
just check       # formatting, tests, and maintenance checks
```

Remotes used by this fork: `origin` → GitHub `colangelo/herdr-max`, `upstream` → GitHub `herdrdev/herdr`, plus a private Gitea mirror that holds the fork's issue backlog.

## agent instructions

If you are an AI agent working on this repository, read [`AGENTS.md`](./AGENTS.md) before making changes and [`CONTRIBUTING.md`](./CONTRIBUTING.md) before opening issues or PRs. Upstream's contribution rules apply to anything destined for `herdrdev/herdr`.

## upstream

Herdr is built full-time and in the open by [@ogulcancelik](https://github.com/ogulcancelik). If this fork is useful to you, the upstream project is the thing worth supporting: [**→ sponsor herdr**](https://github.com/sponsors/ogulcancelik) · [SPONSORS.md](./SPONSORS.md).

## license

Herdr is licensed under the [Apache License 2.0](LICENSE). Fork changes are released under the same license.
