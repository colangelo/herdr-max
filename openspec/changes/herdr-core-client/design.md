# herdr-max as a client of herdr-core: gap analysis

Tracking issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/267

This is the research design for the decision. There is no proposal, spec or task list yet. It answers
one question: what would herdr-max need to run its full feature set on top of upstream's headless
server herdr-core (`hc`), and what would it get for free?

## Sources and pointer conventions

- `hc` 0.1.17, installed at `~/.local/bin/hc`, read on m4m on 2026-11-02 with `HC_SESSION=pilot`.
  Only these commands were run: `--help`, `<cmd> --help`, `--skill` and `schema`. No core was started.
- **schema:** `hc schema` (OpenRPC, 31 methods). `schema:Terminal` is a component, and
  `schema:terminal.send` is a method.
- **skill:** `hc --skill` (the agent manual, `crates/herdr-core/src/SKILL.md` upstream).
- **core:path:** a file in `herdrdev/herdr-core` on GitHub, read through `gh api`. Files read:
  `README.md`, `AGENTS.md`, `docs/commands.md`, `docs/machines.md`, `docs/templates.md`,
  `docs/security.md`, `schema/README.md` and `crates/herdr-core/src/status.rs`.
- **app:path:** a file in `herdrdev/herdr-app`, used only as a hint. Files read: `README.md`,
  `defaults/AGENTS.md` (the mod and model guide), `src/SKILL.md`, `src/model.rs`,
  `src/programs.rs`, `src/mirror.rs`, `src/mirror_sync.rs`, `src/layout.rs` (the `Node` and `Size`
  types) and `src/layout_sync.rs`.
- **docs:X.mdx:** `docs/next/website/src/content/docs/X.mdx` in this repo.
- **INV:topic:** a row of `FORK-INVENTORY.md`.
- **SK:path:** `~/_sync/dev/CONTEXT/SKILLS/path`.
- Anything marked **(inferred)** is a reading of the design and was not seen in a doc or in code.

## 0. The two facts that shape everything

1. **Core draws nothing and has no layout.** core:AGENTS.md § The rule says: *"Core is a runtime,
   not a multiplexer. If a feature only makes sense with a layout or a workspace to return to, it
   does not belong here."* Workspaces, tabs, splits, focus, todos, plugins and notifications will
   never be in core. They belong to a client. herdr-app keeps its spaces, tabs and splits in a core
   document per machine (app:README.md § Work on another machine; app:src/mirror_sync.rs
   *"Latest-state CAS flushing"*; the `doc.get` and `doc.set` methods in core:schema/README.md
   § Documents).
2. **Core status comes only from OSC 7501.** core:AGENTS.md § Rules says: *"Status comes only from
   OSC 7501. No screen detection, no agent registry, no vendor-specific logic. This is the one most
   likely to be helpfully added; don't."* herdr-max status comes from process identification,
   screen manifests and hook reports (docs:agents.mdx § How status works;
   `src/detect/manifests/*.toml`). The `herdr` and `herdr-max` sources never mention 7501: a grep of
   `src` and `docs/next` for `7501` finds nothing.

Today the herdr-max client gets **cells**. The server runs libghostty-vt and sends pane surfaces
and surface deltas (`src/protocol/wire.rs` `ClientShellSnapshot` and `PaneSurfacePane`;
`src/protocol/surface_delta.rs`). Core sends **raw bytes** on an attach stream (core:schema/README.md
§ Attach streams). So a core client must emulate the terminal of every pane itself, as herdr-app does
with `src/vt.rs`, about 100 KB.

### Two shapes for "herdr-max on core"

- **Shape A, direct client.** The herdr-max TUI talks to core alone. It runs a VT emulator for each
  pane and keeps workspaces, tabs and splits in a core document, like herdr-app. Detection, todos,
  plugins and the herdr JSON API need a new home.
- **Shape B, adapter (inferred as the cheaper first step).** The herdr-max server stays and becomes a
  core client. Its PTY layer (`src/pty/backend.rs`, `src/pty/actor.rs`) is replaced by core terminals
  and attach streams. It keeps `AppState`, detection, todos, tasks, plugins, the socket API and the
  TUI protocol. Core then owns the processes, their survival across updates and the remote machines.
  The fleet CLI keeps working almost unchanged.

The "where" column below uses these names. **our client** means TUI code. **adapter/sidecar on core**
means the Shape B server or a small sidecar. **needs core upstream** means core must change.
**drop** means core already covers it and our code goes away.

## 1. Model mapping

| herdr-max noun | Pointer | core noun | Pointer | Note |
|---|---|---|---|---|
| session (named server) | docs:persistence-remote.mdx § Named sessions | session (`HC_SESSION`) | core:docs/security.md § Sessions | Same idea: one server, its own state and socket. Config is shared in both. |
| server | docs:concepts.mdx § Client and server | core plus holders | core:docs/security.md § What runs | Holders own the PTYs, so a core restart or update leaves programs running. |
| workspace / space | docs:concepts.mdx § Workspace | **none** in core | core:AGENTS.md § The rule | herdr-app keeps spaces in the core doc `herdr-app` and labels each terminal `herdr.space=<id>` (app:defaults/AGENTS.md § The model). See § 2d. |
| tab | docs:concepts.mdx § Tab | **none** in core | same | Also kept in the `herdr-app` doc, with the label `herdr.tab=<id>`. |
| pane | docs:concepts.mdx § Pane | terminal | schema:Terminal | A pane shows one terminal. Geometry, zoom and focus have no counterpart. |
| pane id (`w1:p1`, public `terminal_id`) | docs:socket-api.mdx § Identity across restarts and live handoff | address (`t7`, `box/t7`) | schema:Terminal `address`: *"this terminal forever on this core"* | Core addresses never move. Ours are scoped to a server (docs:connecting-machines.mdx § Settings and automation). |
| agent | docs:concepts.mdx § Agent | a terminal whose program reports OSC 7501 (`reported`, `status.app`) | schema:Terminal; core:README.md § Status comes from the program | Core has no agent registry. herdr-app maps the 7501 `app` word to a name (app:src/programs.rs `AGENTS_FIRST`). |
| agent name (`agent rename`) | SK:herdr/reference.md:152 | terminal `name`: unique among live terminals, 1-32 of `a-z 0-9 - _ .`, set at run | schema:terminal.run `name` | Core has **no rename** verb. All 27 live agent names on m4m (`herdr-beta agent list`, 2026-11-02) fit core's rule. |
| agent status (idle, working, blocked, done, unknown, plus `blocked_reason`) | docs:agents.mdx § Blocked state | `status.state` (idle, working, done, blocked, error) plus `kind` (permission, question, auth), `msg`, `progress` | schema:Status | Close match, but only if the program reports. |
| agent session ref (`agent_session`, `restore_argv`) | docs:session-state.mdx § Native agent session restore | **none** | core:docs/security.md § What runs (*"never reruns"*) | Labels could carry it (inferred). |
| machine (saved SSH profile, `--machine`) | docs:connecting-machines.mdx | machine (`machine add`, `--on`, `box/t3`) | core:docs/machines.md | Core merges the remote lists and events into the local core. |
| pane label | `pane.rename` | labels (free-form `k=v`) | schema:terminal.label | Labels are never interpreted. They can be filtered in `ls` and `events`. |
| workspace metadata tokens (`report-metadata`, TTL) | `src/metadata_tokens.rs` | **none** (labels exist per terminal only) | schema:terminal.label | |
| todo / task | INV:todos; `openspec/changes/task-store/proposal.md` | **none** | | A core doc (`doc.set`, 1 MiB per doc, CAS) could hold them (inferred). |
| none | | parent / children, notes, `reach` | skill § Talk, wait, and collect results | Core has an agent tree. herdr-max does not. |
| `agent start --kind`, config | docs:agent-automation.mdx § Agent identity and launch | template | core:docs/templates.md | |
| plugin | docs:plugins.mdx | **none** (herdr-app has JS mods) | app:README.md § Make it yours | |
| client (attached TUI, `client list`) | `src/api/schema/clients.rs` | viewer (an attach stream) | core:schema/README.md § Attach streams | Core does not list viewers. |
| notification, toast | `src/client/shell/notification_center.rs` | notes to the parent agent only | skill § Talk, wait, and collect results | |
| session.json | docs:session-state.mdx § Snapshot restore | `docs.json` (opaque docs) plus saved screens | core:schema/README.md § Documents | |

## 2. Gap table: what herdr-max needs and core lacks

Verdicts: **have**, **partial** or **missing**. "Where" says where the missing part would live. A
`have` row needs nothing, so its "where" is `-`, or `drop` when our code would go away.

### Server runtime

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 1 | Programs run in PTYs that outlive the client | `src/pty/`; docs:session-state.mdx § Live persistence | `terminal.run`; holders (core:docs/security.md § What runs) | have | drop (our PTY layer) |
| 2 | Terminal emulation for display | server-side libghostty-vt; cells to the client (`src/protocol/wire.rs` `PaneSurfacePane`, `src/protocol/surface_delta.rs`) | raw bytes plus a first paint on `terminal.attach` (core:schema/README.md § Attach streams) | partial | our client (VT per pane), or adapter |
| 3 | Resize and size ownership | `pane.resize`; INV:detached-split-size | RESIZE frame; *"The last active viewer sets the size"*; `observe` viewers never resize | have | - |
| 4 | Input delivery (keys, paste, mouse) | `pane.send_input`, `src/client/input/` | DATA on the attach channel; *"The core never encodes keys"* | have | our client encodes keys, as it does today |
| 5 | Terminal queries (XTVERSION, colors, cell size) | `src/pane/xtgettcap.rs`, `src/terminal_theme.rs` | FACTS frame (`schema:Display`), `identity` | have | - |
| 6 | Kitty graphics and images | `src/kitty_graphics/`, `pane.graphics.*` | bytes pass through, nothing parsed (inferred) | partial | our client |
| 7 | OSC 52 clipboard and the toast anchored to its pane | `src/client/clipboard_forwarding.rs`; INV:chrome-clipboard-feedback | bytes pass through, so the client sees OSC 52 (inferred) | partial | our client |
| 8 | Synchronized-output hold with a 200 ms cap | INV:sync-frame-hold | none: the first paint and bytes are raw | missing | our client |
| 9 | Synchronized panes (broadcast typing) | `openspec/changes/synchronize-panes`, `pane.sync` | none | missing | our client (write DATA to each stream) |
| 10 | Several clients, each with its own focus, and `client list` | `src/api/schema/clients.rs` | several viewers allowed; no viewer listing in schema | partial | needs core upstream (viewer list), or adapter |
| 11 | Process facts (shell pid, agent pid, nested job behind a wrapper, `shell_at_prompt`) | `pane.process_info`; `openspec/changes/archive/2026-09-28-workspace-hibernation-primitives` | `pid`, `foreground {pid, name, argv, cwd}`, about twice a second (core:schema/README.md § Events `foreground`) | partial | adapter (walk the local pid tree; inferred) |
| 12 | Working directory | `cwd`, `foreground_cwd` | `cwd`, `foreground.cwd` | have | - |
| 13 | `last_input_at` per pane | hibernation primitives (above) | none | missing | adapter or our client |
| 14 | Named sessions | docs:persistence-remote.mdx § Named sessions | `HC_SESSION` | have | - |
| 15 | macOS: a server that survives logout (per-user context) | INV:macos-server-context | a one-shot launchd job (skill § Status, recovery, and cleanup); logout not covered | partial | pilot first; maybe needs core upstream |
| 16 | macOS TCC: each pane is its own responsible process | `openspec/changes/pane-anchor`, `src/platform/macos/pane_anchor.rs` | holders own programs; no TCC handling seen | missing | needs core upstream, or drop (pilot) |
| 17 | Reaping a server spawned by the client | INV:daemon-reaper | core manages its own start | have | drop |

### Agent status and detection

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 18 | Identify the agent from the process | `src/detect/mod.rs` `identify_agent` | none by rule (core:AGENTS.md); `foreground.name` and `argv` are exposed | missing | adapter |
| 19 | Screen-manifest states (idle, working, blocked) | `src/detect/manifests/*.toml`; docs:agents.mdx § Detection manifests | none by rule | missing | adapter (needs the screen: an `observe` attach, or `hc read` polling) |
| 20 | Hook reports from integrations (Claude hook v12, `pane.report_agent`) | INV:claude-integration; docs:integrations.mdx | only the program's own OSC 7501; no method sets another terminal's status | missing | adapter (hooks write labels or our API), or hooks print 7501 (pilot) |
| 21 | Status vocabulary with a blocked reason | docs:agents.mdx § Blocked state | `Status.state` plus `kind` permission, question or auth | have | - (only if reported) |
| 22 | Background-work mark (braille, one per job) | INV:claude-background-shell → #172; `src/pane/background_agent.rs` | `records`: other 7501 records by id | partial | adapter |
| 23 | Claude attention hint (question raises blocked ahead of the screen) | INV:agent-hints; `openspec/changes/claude-attention-hint` | `blocked` with `kind: question`, if Claude reports it | partial | adapter |
| 24 | `agent explain`, manifest hot reload, remote manifest updates | docs:agents.mdx § Detection manifests | none | missing | adapter |
| 25 | State rollups to tab and workspace | docs:agents.mdx § State rollups | none (no workspaces) | missing | our client (herdr-app does its own: app:src/model.rs) |
| 26 | Agent names, rename and name fallback | `openspec/changes/agent-name-fallback`; SK:herdr/reference.md:152 | `name` is set only at run, with no rename verb | partial | needs core upstream (rename), or a label |
| 27 | Agent session id (Claude `session_id`) | `agent_session` on `pane list` | none | missing | adapter (a hook writes a label; inferred) |
| 28 | Wait for a state | `agent wait --until` | `terminal.wait`: *"until it stops working"* | have | - (only if reported) |
| 29 | Wait for output | `pane.wait_for_output` | `wait --output`: only text printed after the wait starts | have | - |
| 30 | A counter that moves on each state change | `state_change_seq` (`ClientShellAgent`) | `turn`: *"Goes up each time the status leaves working"* | have | - |

### Persistence and restore

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 31 | Layout comes back after a server restart | `session.json`; docs:session-state.mdx § Snapshot restore | `doc.get` / `doc.set` (CAS, survive restart and handover); herdr-app stores layout this way | partial | our client or adapter (write a layout doc) |
| 32 | Processes survive a server update or restart | live handoff, opt-in and experimental (docs:session-state.mdx § Live handoff) | `server.restart`: *"programs keep running, the next core adopts them"*; `hc update` | have | drop (our handoff) |
| 33 | After a reboot, a fresh shell in the last folder | snapshot restore | `keep`: a shell in the last observed folder (core:schema/README.md § Events, last part) | have | - (needs `--keep`) |
| 34 | Agent resume, same agent and same flags (`claude --resume`, settings) | `src/agent_resume.rs`; `openspec/changes/agent-restore-same-agent-same-flags`, `claude-resume-settings-and-scripted-close` | `keep` *"never reruns the task"* | missing | adapter (on a `restarted` event, send the resume line) |
| 35 | Recent screen after a restart | `[experimental] pane_history` (docs:session-state.mdx § Pane screen history replay) | saved screens; `screen: exact/inexact` | have | - (reboot case: pilot) |
| 36 | Layout snapshots and backups (48 kept) | docs:session-state.mdx § Shutdown and snapshot recovery | `docs.json`; a broken file is renamed `.bad` | partial | adapter |
| 37 | Respawn pane, respawn agent | `pane.respawn`, `pane.respawn_agent` (`openspec/changes/archive/2026-10-19-respawn-agent-resume`) | `kill` then `run`; `keep` | partial | adapter |

### Update and handoff

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 38 | Server replacement that keeps panes | `src/handoff_runtime.rs`; INV:handoff-compat | holders, `server.restart`, `hc update` | have | drop |
| 39 | Client stays attached across a server swap and re-execs | `src/client/handoff.rs`; INV:client-handoff | event `seq` restarts per core; *"take a fresh snapshot after reconnecting"* | partial | our client (reconnect and reattach) |
| 40 | Our own release channel (`-ac`, beta, tap) | INV:release-channel, INV:update-manifest | `hc update` from releases.herdr.dev, or `--from <file>` | partial | our client keeps its channel; core comes from upstream |
| 41 | A wire contract that only grows | AGENTS.md § Stable client endpoint contract | *"The API only grows"*; `released.openrpc.json` test (core:schema/README.md) | have | - |

### Layout and organisation

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 42 | Workspaces | docs:concepts.mdx § Workspace | none | missing | our client (doc) |
| 43 | Tabs | docs:concepts.mdx § Tab | none | missing | our client (doc) |
| 44 | Splits, ratios, presets (with main-vertical), balance, swap, zoom | `layout.*`, `pane.split/swap/zoom`; `openspec/changes/add-main-vertical-layout-preset` | none | missing | our client |
| 45 | Move a pane across tabs or spaces; move picker | `pane.move`; `src/client/shell/move_picker.rs` | none | missing | our client |
| 46 | Pinned spaces and agents | `openspec/changes/pin-spaces-and-agents` | none (labels could carry it) | missing | our client |
| 47 | Worktrees | `worktree.*`, `src/client/shell/worktrees.rs` | none (`machine.exec` can run git) | missing | our client or adapter |
| 48 | Branch and ahead/behind per workspace | `ClientShellWorkspace.branch` | none | missing | adapter |
| 49 | Workspace metadata tokens with TTL (the asks count) | `workspace.report_metadata`, `src/metadata_tokens.rs` | none | missing | adapter (doc or our API) |
| 50 | Pane label | `pane.rename` | labels | have | - (label convention) |
| 51 | Focus per client | `focused_*` in `ClientShellSnapshot` | none; herdr-app keeps focus per screen | missing | our client |

### Remote machines

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 52 | Several machines in one window, one agent list | docs:connecting-machines.mdx | `machine add`; one list and one event stream (core:docs/machines.md § What runs where) | have | drop (our federation code) |
| 53 | Install or replace the server on add | docs:connecting-machines.mdx § Add a machine | `machine add --install`, `machine update` | have | - |
| 54 | CLI on a remote machine (`--machine`) | docs:connecting-machines.mdx § Settings and automation | `--on`, `box/t3` addresses | have | - |
| 55 | Remote dormancy: no pings after 10 min idle, so a closed MacBook can sleep | INV:remote-dormancy (issue 260) | core *"retries a lost connection"*; a 15 s heartbeat on event streams; no idle policy documented | partial | pilot; maybe needs core upstream |
| 56 | Client-only attach to a remote server (`herdr --remote`) | docs:persistence-remote.mdx § Remote attach over SSH | `ssh box hc connect`, or a local core plus `machine add` | have | - |
| 57 | Image paste bridge to a remote pane | docs:persistence-remote.mdx (the image clipboard bridge) | `terminal.put`: a file beside the terminal | have | our client calls it |

### Tasks, work panel, issues and asks

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 58 | Pane todos, board and editor | INV:todos; `todo.*` | none | missing | adapter (store) plus our client (panels) |
| 59 | Close or respawn refuses while todos are open (`force`) | INV:close-force | `kill` has no guard | missing | our client or adapter |
| 60 | Task store (file or NATS KV), `task.*`, conflicts | `src/tasks/`; `openspec/changes/task-store` | none (a doc: 1 MiB, CAS) | missing | adapter (keep `src/tasks` as a sidecar) |
| 61 | Work panel (issues and asks views: `issue.list`, `ask.list`) | `src/client/shell/work_panel.rs` | none | missing | adapter plus our client |
| 62 | Stable `pane_key` and `run_id` | task-store proposal § What Changes | the address is permanent per core; labels | partial | adapter |

### Notifications

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 63 | Toasts, notification center, sound, `notification show` from the CLI | `src/client/shell/notification_center.rs`, `notification.*`, `src/sound.rs` | only notes typed into a parent agent | missing | our client plus adapter (an API to post) |
| 64 | Done vs seen per client | `openspec/changes/archive/2026-07-21-notification-read-state` | none; herdr-app tracks it itself (app:src/model.rs) | missing | our client |

### Plugins

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 65 | Plugins: manifest, actions, panes, popups, link handlers, events | docs:plugins.mdx; the `gestore.asks` popup (SK:gestore/asks-plugin/herdr-plugin.toml) | none | missing | adapter (actions, events) plus our client (popups, panes) |
| 66 | Custom command keybindings | docs:plugins.mdx § Keybindings | none | missing | our client |

### CLI and automation used by fleet tools (detail in section 3)

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 67 | JSON socket API with events for every noun | docs:socket-api.mdx | OpenRPC plus `events.subscribe`, for terminals and docs only | partial | adapter (serve our API) |
| 68 | Send a message to another agent | `agent prompt`, `pane send-text`; agent-bell peer messages | `hc send` (paste, Enter, waits ~3 s for a person to pause), notes, `reach` | have | - (a different model) |
| 69 | Type text without pressing Enter; chunked sends | `pane send-text --chunk` (SK:herdr/SKILL.md:56) | `send` always presses Enter; `keys` takes named keys and single characters | partial | needs core upstream, or adapter (write DATA on a stream) |
| 70 | Read with ANSI, or by source (visible, recent, detection) | `pane read --source --format` | `read`: text only, screen plus `--lines` history; `how` is `stitched` for full-screen apps | partial | adapter (or needs core upstream for ANSI) |
| 71 | An env var naming the pane (`HERDR_PANE_ID`, `HERDR_ENV`) | SK:herdr/reference.md:82 | `HC_TERMINAL_ID` | have | - (rename in tools) |
| 72 | Codex app-server panes (`--remote unix://…`) | `openspec/changes/archive/2026-09-27-codex-app-server-panes` | skill: *"Codex must run with `--no-daemon` … or its commands execute outside hc"* | partial | pilot (may conflict) |

### Client UI

| # | Capability | herdr-max today | core today | Verdict | Where |
|---|---|---|---|---|---|
| 73 | Sidebar, agent panel, overflow hints, tab bar, pane chrome, size labels | INV:chrome; `src/client/shell/sidebar.rs`, `tabs.rs`, `openspec/changes/resize-size-labels`, `sidebar-overflow-hints` | none (*"Core draws nothing"*) | missing | our client (exists; needs a new data source) |
| 74 | Copy mode, scroll, search, edit scrollback | `src/client/shell/copy_mode.rs`; `pane.copy_*` | `attach --history`, `read --lines` | partial | our client (scrollback in the client VT) |
| 75 | Command palette, display panes, help, settings, mobile layout, context menus | `src/client/shell/palette.rs`, `display_panes.rs`, `settings_overlay.rs`, `mobile.rs` | none | missing | our client |
| 76 | Config and keybinding reload, theme | `src/client/config_reload.rs` | config holds only `shell`, `notes`, `wait_timeout`, socket | missing | our client |
| 77 | Update notice, release notes, product announcement | `ClientShellSnapshot.update_available`, `release_notes` | `server.status` `version`, `installed_by` | partial | our client |

**Counts: have 24, partial 22, missing 31 (77 rows).**

## 2b. Core has, we lack

"Free" means we get it by being a core client. "Client work" means it needs code on our side to use.

| # | Capability | core | herdr-max today | Value to us | How we'd get it |
|---|---|---|---|---|---|
| 1 | Programs survive core updates and restarts by default, with several holders draining | holders; `server.restart`; `hc update` (core:docs/security.md § Updates and signing) | partial: live handoff is opt-in, experimental, and was limited to 64 panes per batch (docs:session-state.mdx § Live handoff) | high: handoff is our riskiest code (INV:client-handoff, INV:handoff-compat) | free |
| 2 | `run --bg`, `send --wait`, `send --if`, `first_message`, turn semantics | schema:terminal.run, schema:terminal.send | partial: `agent start`, `agent prompt`, `agent wait` | high: fewer races in fleet scripts | free (needs 7501) |
| 3 | `send` waits until nobody has typed for ~3 s and holds a person's keys | schema:terminal.send | none: herdr-compact reads the input box first (SK:herdr-compact/herdr_compact.py:127) | high: fleet sends collide with ac's typing today | free |
| 4 | Parent and children, notes to the parent, `wait --children any/all`, `reach` | skill § Talk, wait, and collect results | none: peers use agent-bell over NATS | high: replaces part of agent-bell and the shepherd watchers | free for agents started with hc |
| 5 | Children can live on another machine and report to a local parent | `hello` `calls_you` and `core` (schema:hello) | none | medium | free |
| 6 | Templates (a role plus a command plus readiness, submit and stop keys) | core:docs/templates.md | partial: `agent start --kind` | medium | free |
| 7 | OSC 7501: status from the program, with several records, `progress`, `msg`, `kind` | core:README.md § Status comes from the program; `crates/herdr-core/src/status.rs` | none: screen manifests plus hooks | high: no screen guessing, no manifest upkeep | free if the agents emit it; otherwise adapter work |
| 8 | `interrupt` reports what it observed (stopped, exited, still_working) | schema:InterruptResult | partial: `send-keys ctrl+c`, then a read | medium | free |
| 9 | Event stream with `seq`, `missed` plus a fresh snapshot, heartbeat, label filters | core:schema/README.md § Events | partial: `events.subscribe` (docs:socket-api.mdx § Event subscriptions) | medium | free |
| 10 | Shared docs with CAS and change events, per machine | core:schema/README.md § Documents | none: `session.json` is private to one server | high: layout, todos and pins shared across clients and Macs | client work |
| 11 | Free-form labels, filterable in `ls` and `events` | schema:terminal.label | partial: pane label, metadata tokens | medium | free |
| 12 | Addresses that never change on a core | schema:Terminal `address` | partial: ids are scoped to a server and remapped on handoff | medium | free |
| 13 | The API through stdin and stdout (`hc connect`, `ssh box hc connect`) | core:schema/README.md § Connecting | partial: SSH API forwarding for `--machine` | medium | free |
| 14 | Custom transports (`--via`: fly, sandboxes) and temporary machines | core:docs/machines.md § Use another way in | none (SSH only) | low | free |
| 15 | `exec` and `file read/write/edit` on any machine, with no terminal | schema:machine.exec, schema:file.* | none | medium: remote edits by agents; git facts for worktrees | free |
| 16 | `keep`: a lost program becomes a shell, with backoff | core:schema/README.md § Events, last part | partial: only after a server restart | medium | free |
| 17 | Attach from any shell, observe only, last active viewer sets the size | `hc attach`, schema:terminal.attach | partial: direct terminal attach (docs:persistence-remote.mdx § Direct terminal attach) | medium | free |
| 18 | `put`: send a file beside a terminal on its machine | schema:terminal.put | partial: the remote image paste bridge only | low | free |
| 19 | A generated OpenRPC schema and a test that refuses breaking changes | core:schema/README.md | partial: frozen digests and fixtures (`tests/fixtures/endpoint-method-shapes-v1.json`) | medium | free |
| 20 | Stable error kinds and exit codes (3, 7, 9, 10, 11) | skill § Status, recovery, and cleanup | partial | medium: simpler fleet error handling | free |
| 21 | Windows core | core:README.md § Install | have (`herdr-windows-x86_64.zip`), but the fleet is macOS only | low | not wanted |
| 22 | herdr-app: workers in the agents sidebar and peek | app:README.md § Find your work | partial: agent panel, no peek | medium | client work |
| 23 | herdr-app: JS mods, layouts as files, sound, telemetry | app:README.md § Make it yours; app:defaults/AGENTS.md § Mods | none (§ 2c compares mods with our plugins) | low: mods run only in the GPUI desktop window, so a TUI cannot run them | not wanted (ideas only) |
| 25 | herdr-app: one world for every client of a machine (spaces, tabs and splits in the core doc `herdr-app`, terminals adopted by label) | app:defaults/AGENTS.md § The model | none: one `session.json` per server | medium: the TUI and the desktop app would show the same spaces (§ 2d) | client work |
| 24 | A small upstream that is moving, while the TUI is in maintenance | core:AGENTS.md (lean rule); FORK-INVENTORY.md (673 legacy commits) | n/a | high: our weekly sync burden (#173) falls when the runtime is upstream's | free |

**Rows: 25.**

## 2c. herdr-app mods against herdr-max plugins

Sources: app:defaults/AGENTS.md § Mods and § The herdr module; docs:plugins.mdx; docs:marketplace.mdx.
The mods gallery facts (1 example, the Tmux prefix, and 0 community mods) come from the coordinator's
read of https://herdr.dev/mods/ on 2026-11-02. This report did not open that page.

Do not confuse the two kinds of "mod". INV:agent-hints and `openspec/changes/claude-attention-hint`
use a *Claude Code* mod: a Claude-side UI hook that tells herdr about a question. A herdr-app mod is
a panel in the desktop window. They share only the word.

| Aspect | herdr-app mod | herdr-max today |
|---|---|---|
| Where it runs | inside the desktop window only (GPUI, through gpui-kit). There is no DOM and no web view. | the herdr server and the TUI client. The UI is terminal cells only. |
| Written in | JavaScript, `~/.config/herdr-app/mods/<name>/main.js` plus `mod.json` (place, size) | any executable, plus a TOML manifest `herdr-plugin.toml` (docs:plugins.mdx § Manifest) |
| What it draws | native elements in a layout block, a pane, a dialog, a popup or a window (`openOverlay`) | a terminal pane, or a popup terminal (docs:plugins.mdx § Panes) |
| Live data | `spaces()`, `agents()`, `terminals()`, `machines()`, `focused()`. The mod re-renders on change and never polls. | the socket API and `events.subscribe`, plus `HERDR_PLUGIN_CONTEXT_JSON` at launch |
| Acting | `run(app command)`, `exec(cmd, {on: machine})`, `hc(args)`, `fetch` | `plugin.action.*`, key bindings, link handlers (docs:plugins.mdx § Keybindings, § Link handlers) |
| Storage | `shared(key)` across mods, `localStorage` per copy, `fs` writes under `~/.config/herdr-app` | none in v1: *"Plugins … should own their files"* (docs:plugins.mdx § Storage) |
| Notifications | `notify({title, body, action})`, drawn by the `toast` mod | `notification.show` (server), drawn by the client |
| Built-in UI | the defaults are mods: spaces, agents, tabs, menu, toast, worktree, peek, confirm | Rust in `src/client/shell/`: sidebar, work panel, todo board, notification center |
| Trust | trusted code with no sandbox | trusted code (docs:plugins.mdx § Trust and security) |
| Sharing | a `RECIPE.md` in a public GitHub repo with the topic `herdr-mod` | a marketplace (docs:marketplace.mdx) |

**Verdict.** Mods are not a target for herdr-max. A TUI cannot run GPUI elements, so none of our UI
can become a mod while we stay in the terminal. What we can learn from them:

- the defaults-are-mods idea (sidebar, agents and tabs written in the same API as user code);
- reactive live data instead of polling;
- `exec` with `on: machine`;
- shared state between mods.

Our closest equivalents are the `gestore.asks` popup plugin (SK:gestore/asks-plugin/herdr-plugin.toml),
the work panel (`src/client/shell/work_panel.rs`) and the sidebar (`src/client/shell/sidebar.rs`). If
ac ever uses the desktop app as well, the asks desk and the work panel are the first candidates to
write again as mods (inferred). Gap row 65 (plugins) stays **missing**: core has no plugins, and the
app's mods do not apply to a TUI.

## 2d. Where the layout lives: share the `herdr-app` document, or keep our own

What herdr-app does (app:defaults/AGENTS.md § The model; app:src/mirror.rs; app:src/layout.rs):

- Each machine keeps its spaces, tabs and splits in that core's doc `herdr-app`, through
  `doc.get` and `doc.set` (schema:doc.get, schema:doc.set; core:schema/README.md § Documents).
- The shape is `Document {spaces: [{id, name, folder, tabs: [{id, name, root: Node}]}]}`.
- `Node` is an n-ary tree. Containers have a row or column `Axis`, and each child has a size of
  `{px}` or `{weight}`. Leaves are terminal blocks (`Content::Terminal {terminal: "t7"}`), mod
  panes, stacks or floats.
- Terminals carry the labels `herdr.space=<id>` and `herdr.tab=<id>`. *"A terminal with those labels
  shows up there"*, including one started by hand: `hc run --keep --label herdr.space=<id> --label
  herdr.tab=<id> -- claude`. A sub-agent belongs to its parent's tab with no pane, and the app can
  peek at it.
- Focus, zoom, recent spaces, settled tabs and seen attention stay on each screen
  (`view.json`, the `View` struct in app:src/mirror.rs).
- On simultaneous edits *"the newest accepted document wins (no merging)"*.

### Option 1: the TUI reads and writes the same document and labels

For:
- The TUI and the desktop app show one world on one core: same spaces, tabs and splits, same
  terminals.
- Agents started with the labels land in the right tab whichever client is open.
- Sub-agents and peek come with the model.
- Our binary split tree with ratios maps onto `Node` (a binary split is a two-child container with
  weights). The reverse direction needs n-ary splits in our layout code, or a flattening (inferred).

Against:
- **The format belongs to upstream's app and has no contract.** core never interprets docs
  (core:schema/README.md § Documents). The app's `Document`, `Node` and `Size` are plain serde types
  that can change in any app release.
- **The app drops what it does not know (inferred).** Its structs have no catch-all field (only
  `flatten` on `Node.kind`), so when the app writes, it removes any extra field we add. Our pins,
  custom labels, number prefixes and worktree grouping cannot live in that doc.
- **Last writer wins.** A TUI edit and an app edit at the same time lose one of them. Today one
  server owns the order of our layout edits.
- Mod panes, floats and stacks in an app layout have no TUI meaning. The TUI must keep them
  untouched and draw a placeholder.

### Option 2: herdr-max keeps its own document (for example `herdr-max`)

For:
- We own the format and its compatibility rules, the same way AGENTS.md § Stable client endpoint
  contract treats our wire.
- No coupling to herdr-app releases.

Against:
- The desktop app does not see our spaces. Our terminals, without `herdr.*` labels, stay out of its
  screens (*"Terminals without the labels … stay out of Herdr's screens"*).
- We also lose the free adoption of agents that other tools start with those labels.

### Verdict: Option 1 for structure, plus our own sibling document for the extras

Recommendation (inferred; the pilot must confirm it):

- **Share** the `herdr-app` document and the `herdr.space` / `herdr.tab` labels for structure only:
  spaces, tabs, splits and which terminal is in which tab.
- **Keep a separate `herdr-max` document** for everything the app would drop: pins, workspace
  metadata tokens, our labels and order, notification read state and todo references. Key it by the
  app's space and tab ids.
- Write the shared doc with CAS and read it again on conflict. Never rewrite parts we do not
  understand.

Two conditions apply. If either fails, fall back to Option 2 and keep the labels, so the app still
adopts our terminals:

1. A round trip through the app keeps every structure edit the TUI made (pilot question 15).
2. Upstream accepts the `herdr-app` document as a versioned, documented format, or at least does not
   object to a second writer (ask Can).

ac is the one who decides whether a shared world with the desktop app is worth this coupling. If he
never plans to open herdr-app, Option 2 is simpler.

## 3. Fleet tooling: CLI verbs and fields we depend on

All calls use `herdr-beta`. Fields in `[]` are read from the JSON answer.

| Verb (flags) | Used by | Fields read | core equivalent | Gap |
|---|---|---|---|---|
| `agent list` | SK:shepherd/shepherd.py:541; SK:gestore/ask_router.py:153; SK:gestore/asks.py:523; SK:gestore/watch/gestore_watch.py:1308; SK:gestore/asks_page/herdr_feed.py:148; SK:herdr-compact/idle_maintain.py:77; SK:herdr-compact/capsule.py:1078; SK:shepherd/references/2026-09-26-restart/herdr_restart.py:29 | [pane_id, name, agent, agent_status, terminal_title_stripped, agent_session.value, agent_session.restore_argv, workspace_id, tab_id, cwd] | `hc ls --json`: address, name, status.state, title, cwd, labels | No `agent` kind, no session id, no workspace or tab, no stripped title |
| `pane list` | ~/.local/bin/agent-bell:1296, 1316; SK:gestore/watch/gestore_watch.py:297 | [pane_id, agent, agent_session.value, terminal_title_stripped, cwd] | `hc ls` | Same as above. agent-bell keys peer inboxes on the Claude session id. |
| `agent get` | SK:herdr-compact/herdr_compact.py:127, 172 | [input_box, agent_session] | `hc get` | `input_box` is derived from detection: missing |
| `agent read`, `pane read` (`--source visible\|recent`, `--format text\|ansi`, `--lines`) | SK:herdr-compact/herdr_compact.py:80; idle_maintain.py:298; hibernate.py:697; capsule.py:1633, 2109 | screen text or ANSI | `hc read --lines N` (text; `--lines 0` is the screen) | No ANSI (idle_maintain and capsule parse ANSI); no `detection` source |
| `agent explain` | SK:herdr-compact/herdr_compact.py:277 | the `state:` line | none | missing (detection) |
| `agent wait --until idle --until done --timeout ms` | herdr_restart.py:103; herdr_compact.py:299 | exit code | `hc wait <t> --timeout s` | Have, if the agent reports 7501. The timeout is in seconds. |
| `agent prompt` | idle_maintain.py:498; capsule.py:1439, 2243 | - | `hc send` | have |
| `agent start --kind claude --pane X --timeout -- args` | SK:herdr-compact/hibernate.py:1013; capsule.py:2124 | [pane_id] | `hc run --bg --name n claude args` or a template | partial: core makes a new terminal; placing it in a pane is the client's job |
| `agent rename` | SK:herdr/reference.md:82, 152 | - | none (name set at `run`) | needs core upstream, or a label |
| `agent focus` | SK:gestore/asks.py:1623 | exit code | none (focus belongs to a client) | our client must answer it |
| `pane process-info --pane` | shepherd.py:1145; herdr_restart.py:41; hibernate.py:209, 404, 979, 1051; capsule.py:1639, 1808 | [process_info: shell_pid, agent_pid, argv, nested job, shell_at_prompt] | `hc get`: pid, foreground {pid, name, argv, cwd} | partial (no shell pid vs agent pid, no nested job) |
| `pane send-keys` (ctrl+c, enter, single characters, `space`) | herdr_compact.py:282, 432, 475; hibernate.py:477, 725; capsule.py:1446 | - | `hc keys <t> ctrl+c enter a …` | have; whether `space` is a key name is a pilot question |
| `pane send-text --chunk` (no Enter) | herdr_compact.py:450; SK:herdr/SKILL.md:56 | - | `hc send` (always presses Enter) | partial |
| `pane run <pane> "cmd"` | hibernate.py:1011; capsule.py:2220 | - | `hc send <shell terminal> "cmd"` | have |
| `pane split --direction --ratio --cwd --no-focus` | hibernate.py:985 | [pane_id] | none | missing (layout) |
| `pane rename` | hibernate.py:1005; capsule.py:2087 | - | `hc label` | partial |
| `tab create --workspace --label`, `tab list`, `tab rename` | capsule.py:2078; SK:gestore/watch/sleep_runner.py:506 | [tabs, tab_id] | none | missing (layout) |
| `workspace create --label --cwd --no-focus`, `workspace list` | hibernate.py:992; asks.py:525 | [workspaces, workspace_id, label] | none | missing (layout) |
| `workspace report-metadata --source --token --ttl-ms` | SK:gestore/asks_page/herdr_feed.py:174 | - | none | missing |
| `notification show --body` | herdr_feed.py:217 | - | none | missing |
| `todo add --pane --priority --link`, `todo list`, `todo done` | gestore_watch.py:1426; SK:herdr/SKILL.md:76-79; SK:gestore/watch/gateway_drill.py:106 | [todos] | none | missing |
| `api snapshot` | hibernate.py:140; capsule.py:1467, 2074 | [snapshot: workspaces, tabs, panes, layout] | `hc ls` plus a layout doc | partial |
| `client list` | ask_router.py:160; asks.py:1393 | [clients] (is ac's window attached, and where) | none | missing (needs a viewer list) |
| `plugin link`, `popup --plugin gestore.asks` | SK:gestore/asks.py:1868; SK:gestore/asks-plugin/herdr-plugin.toml; `src/cli/popup.rs` | - | none | missing |
| `server restart`, `server live-handoff`, `server stop` | SK:herdr/reference.md:283; SK:herdr/updating.md | - | `hc server restart`, `hc update` | have (and safer) |
| `--machine <m> …` | capsule.py:1078, 1439 | - | `--on m`, `m/t7` | have |
| `HERDR_PANE_ID`, `HERDR_ENV` | SK:herdr/reference.md:82; SK:herdr/SKILL.md:78 | - | `HC_TERMINAL_ID` | have (rename it in the tools) |
| `integration install claude` (hooks that call `herdr pane report-*`) | INV:claude-integration; docs:integrations.mdx § Claude Code | - | none | missing: the hooks need a new target (labels, 7501 or our API) |

## 4. Blockers

Ranked. Each one stops herdr-max from working as a core client at all, not only from being complete.

> **Live pilot, 2026-11-02 (m4m, hc 0.1.17, `HC_SESSION=pilot`): blocker 1 is resolved for our two
> main agents.** Claude Code 2.1.316 reports `{"app":"claude-code"}` by itself, and on a real prompt
> core recorded `idle` → `working` → `done`. Codex 0.166.0 reports `{"app":"codex","state":"idle"}`.
> No hook and no manifest were involved. Agents that do not emit 7501 still need an adapter or no
> status. Also measured: `hc server restart` with a program running changed the core pid
> (83043 → 13916) and left the program's pid and output untouched (question 6, restart half). A
> terminal gets the environment of the `hc run` caller, not of the core server. A child started from
> inside a Claude session inherited `CLAUDE_CODE_CHILD_SESSION` and the AI-proxy URL without its key,
> and failed with a 401, so a client must launch terminals with a chosen, clean environment. The pilot
> core and today's herdr server ran side by side with no interference seen (question 14, partly).
> Record: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/267

1. **No agent status for Claude Code and Codex unless they emit OSC 7501** (they do, see above). Every fleet tool reads
   `agent_status`. Core's `wait`, `send` readiness, templates and `interrupt` all key on 7501
   (core:docs/templates.md § Readiness and input). Core refuses screen detection by rule
   (core:AGENTS.md). If the agents do not emit 7501, all detection must run in an adapter that sees
   every screen, and the adapter must also produce the status (gap rows 18-24).
2. **The client gets bytes, not cells.** The TUI must own a VT emulator for every pane (gap row 2),
   or Shape B keeps a server between core and the TUI. Either way, the hot rendering path moves.
   AGENTS.md § Multiplicative performance paths applies.
3. **No layout home and no client verbs.** Workspaces, tabs, splits, focus and pins (gap rows 42-51)
   plus the fleet's `workspace/tab/pane split/api snapshot/client list/agent focus` must be rebuilt
   on a core doc, served by our client or adapter (§ 2d chooses the doc). A CLI for layout must work
   with no window open, so it must edit the doc itself, not ask a running TUI (inferred).
4. **No agent identity beyond a name.** There is no agent kind, no Claude session id and no rename
   (gap rows 26, 27). agent-bell keys peer inboxes on the session id, shepherd and herdr-compact key
   on names, and the resume after a reboot needs `restore_argv` (gap row 34). Hooks must write this
   somewhere core keeps it, such as labels.
5. **No server-side store for todos, tasks, metadata tokens and notifications.** These are
   server-owned today (gap rows 49, 58-63) and fleet tools write them from outside. A core doc
   (1 MiB, CAS) or a sidecar API must replace them before gestore, the asks feed and
   `gestore_watch.py` can run.

Not blockers, but costs: text without Enter (row 69), ANSI reads (row 70), the macOS logout and TCC
behaviour (rows 15, 16), and remote dormancy (row 55). Each can be worked around or upstreamed.

## 5. Questions for the live pilot

Run with `HC_SESSION=pilot` and scratch XDG paths, as core:AGENTS.md § Traps asks.

1. Does Claude Code emit OSC 7501? Does Codex? Note the versions. If not: does `hc run --bg claude`
   time out on readiness (30 s), what does `first_message.sent` say, and do `send` and `wait` still
   work with `status: null`?
2. Can a Claude Code hook make core record a status, by writing OSC 7501 to the pane's tty? Does the
   hook inherit `HC_TERMINAL_ID`, so that `hc label $HC_TERMINAL_ID claude.session=<id>` works?
3. Reboot: do kept terminals come back as shells in their last folders? Does the saved screen show
   (`screen` exact or inexact)? Do docs survive? How long until all are back?
4. CPU and memory (macOS footprint, not RSS): an idle core with 30 terminals, against today's
   herdr-max server. Then the cost of one `observe` attach per terminal for adapter detection,
   against `hc read` polling.
5. Typing echo latency and frame cost through a raw attach stream with a client-side VT, at 1 and at
   15 visible panes (`just bench-render-scale` as the baseline).
6. `hc server restart` and `hc update` with viewers attached: do streams close, how fast does
   reattach work, and does `seq` reset as documented?
7. Remote dormancy: with mbm5 added as a machine and the lid closed, does core keep pinging or
   reconnecting all night (issue 260)? What is its retry schedule?
8. macOS logout: does a core started by the launchd one-shot job survive a logout
   (INV:macos-server-context)?
9. TCC: which process is the responsible one for a privacy prompt raised in an hc terminal? Does the
   answer persist across `hc update`?
10. Codex app-server mode against core's `--no-daemon` rule: do status and commands work, and where
    do Codex commands run?
11. `hc keys`: does it accept `space`, `/` and other single characters? Can we type text without Enter
    in any way other than an attach stream?
12. `hc read` of a Claude Code screen: is the text good enough for our manifests (the
    `bottom_non_empty_lines` regions)? Is the OSC title exposed as `title`, for the `osc_title`
    region?
13. `hc send` quiet rule: when ac types through our client's attach stream, does core hold a
    fleet `send` as documented?
14. Can a pilot core and today's herdr-max server run side by side on one Mac with no interference?
15. Shared layout doc (§ 2d): write a space and a tab into the `herdr-app` doc by hand (`doc.set`
    through `hc connect`), with an extra field. Open herdr-app on the pilot session and make one
    change. Did the app keep our structure, and did it drop the extra field? Does a terminal started
    with `--label herdr.space=<id> --label herdr.tab=<id>` show in the app's tab?

## Appendix: what was not read

- herdr-app beyond the files listed. `herdr.d.ts`, the mod sources and the shape of the layout
  files were not read. The herdr.dev/mods page was not opened (its facts come from the coordinator).
- The OSC 7501 wire format. Only the doc comments of core's `status.rs` were read.
- The core source beyond `status.rs`, for example `viewers.rs` and `terminals.rs`.
