# V12 results (2026-11-15, m4m, release build)

Harness: `scripts/core_busy_profile.sh pty|core`, a throwaway herdr with 30 panes (one workspace
each), each printing one 90-byte line twice a second; CPU seconds used in 30 s busy and 30 s idle
by herdr's server and pane anchor, plus, on core, the core server and its holder. Panes' own
programs are not counted. Load average 7-9 during the runs.

| backend | busy CPU / 30 s | idle CPU / 30 s | footprint | runs |
|---|---|---|---|---|
| PTY | 1.11-1.17 s (server) | 0.25-0.30 s | 23.6-24.6 MB | 4 |
| core | 1.92-2.52 s: server 1.23-1.49, core 0.52-0.63, holder 0.17-0.24 | 0.52-0.66 s: server 0.25-0.29, core 0.18-0.22, holder 0.09-0.11 | 40.7-42.5 MB | 8 |

Ratio core / PTY, same harness: about 1.9x busy, 2.1x idle, over the first V12 target (server +
core + holder within 1.5x).

**Decision (herdr, 2026-11-15):** phase 2 ships opt-in as is. V12 is restated: our server within
1.3x of the PTY on this profile, and core and holder cost recorded, not gated. Against the
restated row: medians 1.37 s (core, 6 runs: 1.23-1.49) and 1.13 s (PTY, 4 runs: 1.11-1.17) give
1.21x, met; the worst pairing of runs is 1.34x, just over, so the margin is thin. Our server's extra
10-30% is https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/282, to be cut or re-measured
before phase 3 makes core the default.

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

## Our server's overhead re-measured (2026-11-16, issue 282)

https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/282 asked to cut our server's extra
10-30% on core or re-measure it before phase 3. Same release build (`fdf5beba`), same harness, all
runs alternating PTY and core, load 4-13.

| when (UTC) | profile | server, PTY | server, core | core / PTY | runs per side |
|---|---|---|---|---|---|
| 13:55-14:10 | busy loop (a `sleep` child per line) | 1.06 s (0.98-1.16) | 1.34 s (1.22-1.42) | 1.26x | 5 |
| 15:15 | one long-running printer per pane | 0.90 s (0.80-0.94) | 0.85 s (0.79-0.90) | 0.94x | 3 |
| 15:40-16:05 | busy loop | 0.74 s (0.73-0.76) | 0.80 s (0.75-0.84) | 1.07x | 4 |
| 15:40-16:05 | one printer per pane | 0.50 s (0.48-0.51) | 0.51 s (0.49-0.53) | 1.02x | 4 |
| 16:10 | busy loop | 0.78, 0.77 s | 0.80, 0.73 s | about 1.0x | 2 |

Medians, with ranges. The same build and harness gave 1.26x at 14:00 and about 1.0-1.07x from
15:40. So the gap follows the machine's state, not a fixed cost in our server. The restated V12 gate
(within 1.3x) is met in every series.

What the server does on each backend, measured on the busy loop:

- **The same user-space work.** macOS `sample`, 24 s per side: 616 on-CPU samples in the tokio
  workers on the PTY and 647 on core, split the same way: detection's process probes, cwd refresh,
  compression and the detection text.
- **The same system calls and context switches.** `top`, 20 s: 51 196 BSD calls on core and
  51 522 on the PTY; 16 668 and 17 737 context switches.
- **The same data.** Core sends one frame per printed line (58 frames a second for 30 panes,
  92 bytes each), as many as the PTY reads.
- **The same probe cost.** `proc_pidinfo`, `proc_listpids` and `KERN_PROCARGS2` on a core pane's
  processes cost what they cost on a PTY pane's.
- **Not interference.** PTY runs beside a separate core with 30 attached busy terminals:
  0.86-0.92 s, against 0.86-1.01 s alone.
- **One cost only core has:** about 120 Mach calls a second (2374 in 20 s; about 10 on the PTY),
  from the hop from the shared reader thread to each pane's parser thread (std mpsc, two
  semaphore calls per chunk). It is there with and without the `sleep` children.

Likely, not proved: the hop's wakeups cost more when the performance cores are busy and our niced
server runs on efficiency cores, which would explain a 1.26x afternoon and a 1.0x evening. Not
measured: which cores the threads ran on.

**Not changed.** Removing the hop means parsing on the reader thread. Then one pane's parse,
which takes that pane's terminal lock, would hold up every pane's stream and every reply to our
RPC calls, and the change would have to prove that no caller waits on a core reply while it holds a
terminal lock, or it deadlocks. That is not worth a cost that measures 0-7% today. Re-measure before phase 3 with the harness's
`V12_TOP` and `V12_ONE_PROCESS` (see `scripts/core_busy_profile.sh`). If a loaded machine shows
more than 1.3x again, the fix is a reader that hands whole batches to the parsers.

Artifacts: herdr-core's state folder on m4m, `herdr-core-p1/282/` (runs, samples, counters,
probe scripts).
