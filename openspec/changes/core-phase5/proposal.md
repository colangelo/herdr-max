## Why

ac, 2026-11-22: "complete step 3, 4 and 5 today, we have time". Step 5 of the herdr-core migration
(`openspec/changes/herdr-core-client/design.md` § 6.4) replaces herdr-max's own SSH remote (`src/remote/`,
`herdr-beta --remote ac-mbm5`, saved machines) with herdr-core machines (`hc --on`, `ac-mbm5/t7`). That also gives
the goal of § 4d: harnesses in VMs, driven from herdr-max. Record:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/300.

Three facts shape it, two of them measured for this change (`probe/README.md`):

1. **A core machine link is never idle.** With no client and no viewer, the local core polls the other core every
   3 s, plus a 15 s heartbeat: about 1 400 round trips an hour. herdr's 5 s ping alone kept ac-mbm5 in dark wakes
   until a thermal panic (#260). Core has no setting or method to pause a link.
2. **Core retries a link through whatever command it was added with.** A `--via` command that refuses at once
   costs only a local process start, every 30 s at most. So a herdr-owned link command can be the dormancy gate
   with core used as shipped.
3. **Two herdr servers would view one terminal.** ac-mbm5's own herdr owns its panes' layout and detection. Core
   gives a terminal's size to its last active viewer, so m4m must view without taking the size.

## What Changes

- **Machines are core machines in the primary core.** `herdr machine add --core <target>` runs `hc machine add`
  in the person's terminal with herdr's link command as `--via`. Core owns the connection, retries, the merged
  terminal list and `--on`.
- **Every dial passes a lease gate.** `herdr core-link <name>` dials ssh only while a lease is in force: a TUI
  client's lease from human input (#260's clock, 10 min), a fleet CLI call's lease, or `herdr machine wake`.
  Restores, live handoffs, core starts and agents' own `hc` calls never hold one. When the last lease ends, the
  link closes and core's retries refuse locally.
- **A herdr-max pane can run on a machine.** `--on <machine>` on split, tab, workspace and agent start, behind
  an advertised API capability. The pane keeps its herdr id, gains optional `machine` and `core_address` fields,
  carries an ownership label the other Mac's herdr never matches, gets no `HERDR_*` integration variables, and
  waits in place while the machine is dormant or unreachable.
- **Another Mac's herdr panes are shown by a view server.** `herdr server --view <machine>` projects the far
  side's `herdr-app` document (the spaces herdr-max already publishes) and attaches to its terminals without
  taking their size. The TUI reaches it as one more endpoint; `herdr --remote core:<machine>` opens it in a tab.
  Structural actions and `herdr --machine <name>` go to the far side's own herdr CLI through `machine.exec`.
- **SSH dials are classified and announced.** The link reuses the old remote's ssh options and ControlMaster,
  never prompts in the background, and closes its gate on a Tailscale SSH check banner, a stalled Touch ID or an
  auth failure until `herdr machine login`. `herdr machine status --json` gives agents what to announce.
- **The old remote stays.** `herdr --remote <host>`, saved herdr machines and `--machine` to them are unchanged.
  `[remote] prefer = "ssh" | "core"` (default `"ssh"`) decides a shared name. Phase 6 flips it and deletes the
  ssh path.
- **herdr never installs or updates hc**, here or there.

## Capabilities

### New Capabilities

- `core-machine-link`: adding a core machine, the link command, leases and dormancy, failure classes, login, status.
- `core-machine-panes`: a herdr-max pane on a core machine: spawn, label, environment, process facts, close, restore.
- `core-machine-view`: the view server for another Mac's herdr, its size policy, layout, actions and the fleet CLI
  over the link.

### Modified Capabilities

None. The old remote's behaviour is unchanged (core-machine-view's fallback requirement states it).

## Impact

- New: `src/core_machine/` (link command, leases, gate state, `machine` CLI verbs for core), `ServerRole` in the
  server entry, `ClientEndpointId::Core`, `Host` on `CorePaneIo`.
- Changed: `core_backend` spawn and labels, pane JSON (two optional fields), API capability list, `cli/target.rs`
  resolution, the client's dormancy clock (writes a lease).
- No change to `src/remote/` behaviour, the generation 1 client endpoint codecs, or herdr-core.
- Docs: `docs/next` pages for core machines, dormancy and the announcement rule.
