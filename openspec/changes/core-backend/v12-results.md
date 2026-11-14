# V12 results (2026-11-15, m4m, release build)

Harness: `scripts/core_busy_profile.sh pty|core`, a throwaway herdr with 30 panes (one workspace
each), each printing one 90-byte line twice a second; CPU seconds used in 30 s busy and 30 s idle
by herdr's server and pane anchor, plus, on core, the core server and its holder. Panes' own
programs are not counted. Load average 7-9 during the runs.

| backend | busy CPU / 30 s | idle CPU / 30 s | footprint | runs |
|---|---|---|---|---|
| PTY | 1.11-1.17 s (server) | 0.25-0.30 s | 23.6-24.6 MB | 4 |
| core | 1.92-2.52 s: server 1.23-1.49, core 0.52-0.63, holder 0.17-0.24 | 0.52-0.66 s: server 0.25-0.29, core 0.18-0.22, holder 0.09-0.11 | 40.7-42.5 MB | 8 |

Ratio core / PTY, same harness: about 1.9x busy, 2.1x idle. The V12 target (within 1.5x) is
**not met**.

Where it goes:

- Our server: 1.23-1.49 s on core against 1.11-1.17 s on the PTY, 10-30% more (two threads per
  pane and one more channel hop per chunk).
- Core and its holder: 0.70-0.84 s busy. Measured alone today (`core30.sh` and `attach30.py`, kept in herdr-core's state folder on m4m under `herdr-core-p1/v12/`; 30 terminals run with
  `hc run --bg`), core and holder use 0.50 s with no viewer, 0.54 s with 30 observe attaches and
  0.59 s with 30 full attaches. So about 0.5 s is core keeping its own screen for each terminal,
  which Shape B adds by design (core and herdr each parse every pane's output), and about 0.2 s
  more appears when herdr is the viewer.
- Not the cause, measured: credit per chunk (batched to 64 KiB, no change, kept: fewer wakeups)
  and the events subscription (turned off in a local build, no change).

Phase 0's Q4 figure (core and holder 0.31 s busy, +0.07 s for attaches) does not reproduce today
on the same load: 0.50 s with no viewer.

Render bench (`just bench-render-scale`, `bench-render-phase2.txt` against
`openspec/changes/runtime-seam/bench-after.txt`): median ratio 0.93, range 0.85-1.00, no row
slower; 1 vs 15 panes scales as before. The bench started at load 17 (above the 14 rule); a busy
machine only slows it, so the result stands.
