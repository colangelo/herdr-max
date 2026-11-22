## Ground rules

Planned 2026-11-22 (herdr-core, for herdr; https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/300).
Verifier rows V1-V25 are in design.md § Verifier. Each PR is shippable alone, off until chosen (design D8), and
ends in a check.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env CARGO_BUILD_JOBS=6
NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`. Live tests use scratch `HC_SESSION`s and
scratch XDG directories, a local `--via` loop and a fake `ssh` first on `PATH` (`probe/`); no test dials a real
Mac, runs `hc update` or `hc machine update`, or touches ac's `default` or `herdr` core. A test Claude Code runs
only with `--settings ~/.config/claude/direct-noconnectors.settings.json`.

Order: A, B, C, D, E, then rollout R. C and D are independent after B.

## 0. Before code

- [ ] 0.1 Roundtable on design.md § Roundtable questions; findings folded into design.md
- [ ] 0.2 Verifier review by herdr (the lead)
- [ ] 0.3 Gitea sub-issues for PR A-E (`[fork]` titles), each linking #300
- [ ] 0.4 Security review request for D11 (the shared link) to the security agent, `--wake`

## PR A. The link command and core machines (no dormancy yet)

- [ ] A.1 Read the Tailscale SSH check banner text and where ssh prints it from Tailscale's docs or source (no
  dial); record it in design D10
- [ ] A.2 `src/core_machine/`: `MachineName`, `target.json`, `gate.json` types parsed at the boundary; the ssh argv
  builder reusing `attach.rs`'s managed config and ControlPath helpers (D10)
- [ ] A.3 `herdr core-link <name>`: ssh with inherited stdin/stdout, stderr watched, failure classes (D10); a lease
  check that, until PR B, treats every machine as leased
- [ ] A.4 `herdr machine add --core`, `rm --core`, `login`, `status [--json]` for core machines; refusal without a
  terminal; no `--install` (V23)
- [ ] A.5 Loop harness `scripts/core_machine_live_test.sh`: two scratch cores, fake `ssh`, the probe's byte log;
  V8, V23
- [ ] A.6 Check: `just check` 0, `just core-live-test` 0, CI green on Linux, macOS and Windows

## PR B. Leases and dormancy (#260 on core machines)

- [ ] B.1 Lease files (`leases/<holder>`, write-then-rename, expiry) and the gate's lease check (D3); V1, V5
- [ ] B.2 The client writes `client-<pid>` from `InputClock`, at most once a minute; V24
- [ ] B.3 `core-link` re-reads leases every 5 s and ends ssh; `herdr machine wake`; V2, V3, V7
- [ ] B.4 Wake: the first input that reaches a server for a machine's pane, or a view server's first client, calls
  `machine.connect`; V4
- [ ] B.5 Check: `just check` 0, `just core-live-test` 0, CI green

## PR C. A herdr-max pane on a machine

- [ ] C.1 `Host` on `CorePaneIo`; pid facts, sweep and close by host (D5); V12
- [ ] C.2 Spawn with `on`; label `<label>@<core id>`; integration variables stripped; V9, V10
- [ ] C.3 `CoreWaiting` causes `machine_dormant` and `machine_unreachable`; `machine.list` poll while a lease holds;
  input held and wake on first key; V6, V11
- [ ] C.4 API `on` and capability `spawn_on_machine`; CLI `--on`; pane JSON `machine`, `core_address`; V13
- [ ] C.5 New pane dialog: a machine choice, listing core machines with their gate (UI style per `docs/ui-style.md`,
  help entry if a key is bound)
- [ ] C.6 `docs/next`: running a pane on a machine, what a remote pane lacks (hooks, detached-job sweep)
- [ ] C.7 Check: `just check` 0, `just core-live-test` 0, `just bench-render-scale` 1 and 15 panes reported, CI green

## PR D. The view server

- [ ] D.1 `ServerRole` and `herdr server --view <machine>`; the role's boundaries per D6's table, each a match on the
  role
- [ ] D.2 Document parser with limits; layout projection; `doc` and `missed` handling; V15, V16
- [ ] D.3 Follow-size attach; cropped or padded drawing with the remote size in the border; V14 (before merge)
- [ ] D.4 `ClientEndpointId::Core`, endpoint transport that starts the view server; `herdr --remote core:<m>`;
  exit 10 min after the last client; V17
- [ ] D.5 Both-present notice and `[remote] prefer`; V21, V22
- [ ] D.6 Check: `just check` 0, `just core-live-test` 0, CI green

## PR E. Actions and the fleet CLI over the link

- [ ] E.1 Remote herdr path and capability probe per link establishment (D7)
- [ ] E.2 `--machine` through `machine.exec` with the `cli-<pid>` lease; no retry after send; V18, V19
- [ ] E.3 View actions (new tab, split, close pane, rename, agent start) as argv `machine.exec` calls; V20
- [ ] E.4 `docs/next`: core machines, dormancy, `machine status` gates, the announcement rule, the fallback
- [ ] E.5 Check: `just check` 0, `just core-live-test` 0, CI green

## R. Rollout (both Macs in one step)

- [ ] R.1 Open question 1: ac-mbm5's herdr publishes its spaces to its `default` core (ask on the desk if a config
  change is needed there)
- [ ] R.2 Announcement S1 through the gestore desk: "herdr-core needs ssh ac@ac-mbm5 from m4m's Herdr Core now, to
  add ac-mbm5 as a core machine; later dials reuse that link while ac is active". Wait for ac's acknowledgement
- [ ] R.3 Beta with A-E on m4m and ac-mbm5 by live handoff, panes kept; then ac runs `herdr machine add --core
  ac-mbm5` on m4m himself (S1); record the 1Password prompt's app name (open question 3)
- [ ] R.4 V25: one night with the view attached; `pmset -g log` on ac-mbm5 read by mac-m5; results on #300
- [ ] R.5 Close #300 after V25 passes in a running beta, with the check described
