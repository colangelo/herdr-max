# V22 results: the phase 3 CPU gate (D11), 2026-11-22, m4m, release build

Task M of this change (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/295). Both gates are met in
both profiles, so PR E (the batch-handing reader) is not needed.

| profile | runs a side | server, PTY | server, core | gate 1: core / PTY (limit 1.3x) | noise band | core total | gate 2: / phase 2 (limit 1.2x) |
|---|---|---|---|---|---|---|---|
| default (a `sleep` child per line) | 16 | 1.07 s (0.96-1.24) | 1.275 s (1.15-1.43) | **1.19x, met** (1.22x after 8) | 1.07 | 1.945 s (1.82-2.15) | **0.92x, met** (phase 2 median 2.115 s) |
| `V12_ONE_PROCESS=1` | 8 | 0.73 s (0.67-0.89) | 0.805 s (0.70-0.90) | **1.10x, met** | 1.03 | 1.30 s (1.13-1.45) | **1.01x, met** (phase 2 median 1.29 s) |

Busy CPU over 30 s, medians with ranges. The second running core costs 0.02-0.04 s per 30 s while idle, with one
idle terminal. That is about 0.1% of one CPU core, and our server shows no measurable change.

## Versions

- herdr: `415448a2` (origin/master, after PR C and PR D), `cargo build --release --locked`, version string
  `herdr 0.9.5`. The local build is not a beta, so the string has no beta suffix. The runs used a copy of the
  binary taken from the shared target right after the build.
- herdr-core: `hc 0.1.21`.
- macOS 27.2.1 (26B131), m4m.

## Method

The harness is `scripts/core_busy_profile.sh pty|core`, unchanged from phase 2. It starts a throwaway herdr in
session `v12` with its own XDG folders, so its core session is the scratch `herdr-v12`. The herdr has 30 panes,
one workspace each. Each pane prints one 90-byte line twice a second. The harness counts the CPU seconds used in
30 s busy and 30 s idle by herdr's server and pane anchor, and on core also by the core server and its holder.
`V12_TOP` records the server's context switches, BSD calls and Mach calls over 20 s of the busy window (`top -l 2
-s 20`; each counter is the difference between the two samples).

D11 as written:

- The whole run is one script, `driver.sh`. It ran under `lockf -k /tmp/m4m-heavy.lock nice -n 15` with
  `CARGO_BUILD_JOBS=6`, the build included, so no cargo build overlapped any run. The only load the run added was
  one herdr server, its 30 printers and `top`.
- Before each run, the script waited until the 1-minute load was below 14. It waited once, for 30 s, before
  default run 16 PTY. Load before each run ranged from 4.3 to 13.4.
- PTY and core alternate in ABBA order: odd runs start with the PTY and even runs start with core, so slow drift
  falls on both sides.
- Noise band: the larger of the PTY server medians of runs 1-4 and 5-8, divided by the smaller. When the core
  ratio is within that band of 1.3x, eight more runs a side decide.
- Gate 2 reference: the median of phase 2's own raw core totals for the same profile. Default: the 10 runs that
  are kept, 1.90-2.52 s, median 2.115 s (`v12-busy.txt` and `v12-busy-batched.txt` from V12 on 2026-11-15, and
  `282/baseline.txt` from 2026-11-16). `core-backend/v12-results.md` cites 1.92-2.52 s over 8 runs, and only 5 of
  those raw runs are kept. One printer: phase 2's 3 runs in `282/oneproc.txt`, 1.22-1.35 s, median 1.29 s.

## Default profile

After eight runs a side the ratio was 1.218x. The PTY medians were 1.07 s for runs 1-4 and 1.00 s for runs 5-8,
so the band was 1.07. The ratio was inside the band of 1.3x (1.215-1.391), so eight more runs a side ran. Over
all 16 runs the ratio is 1.19x.

| run | side | load before | load after | server busy | total busy | core | holder | server idle | total idle | footprint MB | csw | BSD | Mach |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | pty | 10.87 | 6.98 | 0.96 | 0.96 | - | - | 0.26 | 0.26 | 23.9 | 16120 | 53273 | 4 |
| 1 | core | 6.98 | 5.99 | 1.28 | 1.99 | 0.51 | 0.20 | 0.22 | 0.45 | 40.6 | 17652 | 51733 | 2376 |
| 2 | pty | 5.48 | 4.95 | 1.08 | 1.08 | - | - | 0.23 | 0.24 | 22.7 | 15871 | 52212 | 4 |
| 2 | core | 5.99 | 5.48 | 1.35 | 2.09 | 0.54 | 0.20 | 0.28 | 0.58 | 41.8 | 18384 | 52952 | 2360 |
| 3 | pty | 4.95 | 5.01 | 1.06 | 1.06 | - | - | 0.29 | 0.30 | 23.7 | 15475 | 51412 | 4 |
| 3 | core | 5.01 | 13.39 | 1.21 | 1.89 | 0.49 | 0.19 | 0.20 | 0.44 | 40.9 | 15342 | 47638 | 2326 |
| 4 | pty | 8.56 | 10.03 | 1.13 | 1.13 | - | - | 0.23 | 0.23 | 23.9 | 15669 | 51073 | 8 |
| 4 | core | 13.39 | 8.56 | 1.28 | 2.04 | 0.55 | 0.21 | 0.30 | 0.54 | 40.8 | 17737 | 51491 | 2370 |
| 5 | pty | 10.03 | 7.83 | 1.07 | 1.07 | - | - | 0.29 | 0.29 | 22.9 | 16274 | 52793 | 5 |
| 5 | core | 7.83 | 6.68 | 1.29 | 2.02 | 0.53 | 0.20 | 0.24 | 0.45 | 39.4 | 17956 | 51836 | 2370 |
| 6 | pty | 6.44 | 5.40 | 1.00 | 1.00 | - | - | 0.22 | 0.22 | 22.8 | 16643 | 53842 | 4 |
| 6 | core | 6.68 | 6.44 | 1.19 | 1.86 | 0.49 | 0.18 | 0.26 | 0.53 | 38.6 | 16062 | 48916 | 2370 |
| 7 | pty | 5.40 | 5.46 | 0.96 | 0.97 | - | - | 0.25 | 0.25 | 24.8 | 15506 | 51878 | 6 |
| 7 | core | 5.46 | 7.77 | 1.15 | 1.82 | 0.48 | 0.19 | 0.24 | 0.48 | 41.7 | 17201 | 50719 | 2346 |
| 8 | pty | 8.51 | 8.71 | 1.00 | 1.00 | - | - | 0.24 | 0.24 | 24.8 | 16164 | 52793 | 12 |
| 8 | core | 7.77 | 8.51 | 1.23 | 1.94 | 0.52 | 0.19 | 0.29 | 0.58 | 41.4 | 17943 | 52700 | 2376 |
| 9 | pty | 5.68 | 5.86 | 1.24 | 1.24 | - | - | 0.31 | 0.31 | 22.8 | 15454 | 50918 | 6 |
| 9 | core | 5.86 | 6.90 | 1.27 | 1.92 | 0.47 | 0.18 | 0.27 | 0.56 | 41.3 | 18373 | 53398 | 2382 |
| 10 | pty | 7.35 | 10.74 | 1.07 | 1.07 | - | - | 0.27 | 0.28 | 23.8 | 17379 | 53849 | 3 |
| 10 | core | 6.90 | 7.35 | 1.29 | 2.02 | 0.53 | 0.20 | 0.26 | 0.51 | 42.2 | 18447 | 52803 | 2394 |
| 11 | pty | 10.74 | 9.26 | 1.07 | 1.07 | - | - | 0.28 | 0.28 | 22.9 | 18371 | 54695 | 0 |
| 11 | core | 9.26 | 7.90 | 1.20 | 1.86 | 0.48 | 0.18 | 0.26 | 0.51 | 40.9 | 20271 | 55115 | 2390 |
| 12 | pty | 6.35 | 5.33 | 1.12 | 1.12 | - | - | 0.28 | 0.28 | 23.9 | 17426 | 53517 | 4 |
| 12 | core | 7.90 | 6.35 | 1.24 | 1.89 | 0.47 | 0.18 | 0.26 | 0.47 | 41.9 | 19759 | 53367 | 2382 |
| 13 | pty | 5.33 | 5.74 | 1.17 | 1.17 | - | - | 0.28 | 0.28 | 23.6 | 17621 | 53234 | 2 |
| 13 | core | 5.74 | 5.94 | 1.29 | 1.95 | 0.48 | 0.18 | 0.31 | 0.55 | 40.9 | 19827 | 53823 | 2392 |
| 14 | pty | 4.32 | 8.62 | 1.11 | 1.12 | - | - | 0.29 | 0.29 | 22.6 | 17300 | 53460 | 4 |
| 14 | core | 5.94 | 4.32 | 1.43 | 2.15 | 0.54 | 0.18 | 0.30 | 0.55 | 43.1 | 18562 | 52076 | 2378 |
| 15 | pty | 8.62 | 5.80 | 1.13 | 1.13 | - | - | 0.28 | 0.28 | 24.8 | 17293 | 53470 | 6 |
| 15 | core | 5.80 | 6.01 | 1.25 | 1.84 | 0.43 | 0.16 | 0.28 | 0.52 | 39.6 | 20898 | 55174 | 2390 |
| 16 | pty | 10.25 | 6.85 | 1.05 | 1.05 | - | - | 0.22 | 0.22 | 23.8 | 16895 | 52360 | 2 |
| 16 | core | 6.01 | 14.29 | 1.29 | 1.96 | 0.49 | 0.18 | 0.28 | 0.50 | 42.0 | 19382 | 53148 | 2390 |

Medians over 16 runs a side:

- Our server: 1.275 s on core and 1.07 s on the PTY, 1.19x. The run-by-run ratios range from 1.02x to 1.33x.
- Core and its holder: 0.49 s and 0.185 s.
- Idle (30 s): our server 0.265 s on core and 0.275 s on the PTY. Total on core 0.515 s.
- Footprint: 41.1 MB on core and 23.75 MB on the PTY.
- Server counters over 20 s: 18 379 context switches on core and 16 459 on the PTY; 52 752 and 53 014 BSD calls;
  2377 and 4 Mach calls.

## One printer per pane (`V12_ONE_PROCESS=1`)

The PTY medians were 0.75 s for runs 1-4 and 0.73 s for runs 5-8, a band of 1.03. The ratio of 1.10x is outside
the band of 1.3x (1.27-1.34), so eight runs a side decide.

| run | side | load before | load after | server busy | total busy | core | holder | server idle | total idle | footprint MB | csw | BSD | Mach |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | pty | 8.71 | 8.87 | 0.78 | 0.79 | - | - | 0.26 | 0.26 | 23.7 | 14891 | 31774 | 0 |
| 1 | core | 8.87 | 9.76 | 0.81 | 1.32 | 0.31 | 0.20 | 0.25 | 0.52 | 41.7 | 17394 | 33126 | 2404 |
| 2 | pty | 9.32 | 6.98 | 0.72 | 0.73 | - | - | 0.29 | 0.29 | 24.0 | 15904 | 34447 | 0 |
| 2 | core | 9.76 | 9.32 | 0.81 | 1.34 | 0.32 | 0.21 | 0.31 | 0.64 | 39.5 | 17373 | 33860 | 2398 |
| 3 | pty | 6.98 | 7.16 | 0.89 | 0.89 | - | - | 0.30 | 0.30 | 23.7 | 16402 | 34763 | 0 |
| 3 | core | 7.16 | 8.88 | 0.80 | 1.28 | 0.29 | 0.19 | 0.24 | 0.49 | 39.5 | 18174 | 34484 | 2396 |
| 4 | pty | 7.75 | 6.35 | 0.67 | 0.67 | - | - | 0.23 | 0.23 | 23.9 | 15248 | 32538 | 0 |
| 4 | core | 8.88 | 7.75 | 0.90 | 1.45 | 0.33 | 0.22 | 0.26 | 0.55 | 41.8 | 17592 | 32846 | 2400 |
| 5 | pty | 6.35 | 7.75 | 0.72 | 0.72 | - | - | 0.29 | 0.30 | 23.8 | 15089 | 32544 | 0 |
| 5 | core | 7.75 | 7.90 | 0.72 | 1.17 | 0.27 | 0.18 | 0.28 | 0.54 | 41.8 | 16572 | 30773 | 2408 |
| 6 | pty | 5.54 | 5.93 | 0.72 | 0.72 | - | - | 0.30 | 0.30 | 22.7 | 15417 | 32166 | 0 |
| 6 | core | 7.90 | 5.54 | 0.86 | 1.38 | 0.31 | 0.21 | 0.27 | 0.51 | 39.5 | 16119 | 29831 | 2412 |
| 7 | pty | 5.93 | 5.55 | 0.82 | 0.82 | - | - | 0.31 | 0.31 | 23.5 | 14806 | 31791 | 0 |
| 7 | core | 5.55 | 8.01 | 0.70 | 1.13 | 0.26 | 0.17 | 0.27 | 0.54 | 41.3 | 16200 | 30351 | 2414 |
| 8 | pty | 6.67 | 5.68 | 0.74 | 0.74 | - | - | 0.29 | 0.29 | 23.9 | 15348 | 32617 | 0 |
| 8 | core | 7.77 | 6.67 | 0.78 | 1.24 | 0.27 | 0.19 | 0.31 | 0.58 | 43.5 | 16473 | 31281 | 2422 |

Medians over 8 runs a side:

- Our server: 0.805 s on core and 0.73 s on the PTY, 1.10x. The run-by-run ratios range from 0.85x to 1.34x.
- Core and its holder: 0.30 s and 0.195 s.
- Idle (30 s): our server 0.27 s on core and 0.29 s on the PTY. Total on core 0.54 s.
- Footprint: 41.5 MB on core and 23.75 MB on the PTY.
- Server counters over 20 s: 16 973 context switches on core and 15 298 on the PTY; 32 064 and 32 541 BSD calls;
  2406 and 0 Mach calls.

## The idle cost of a second core

`second_core_idle.sh` copies the setup of `scripts/core_default_session_live_test.sh`. It runs a release build's
default herdr session with its own XDG folders, so its scratch core sessions are `default` and `herdr`. Both
states have two idle core panes (`/bin/sh`), and our server is killed and restarted before each one, so the
restart is part of both. Each state has 4 windows of 30 s.

| state | our server | core `default` + holder | legacy core `herdr` + holder |
|---|---|---|---|
| A: both panes in `default`, one core | 0.02-0.04 s | 0.03-0.05 s | not running |
| B: pane G re-adopted from the legacy `herdr` core, two cores and a secondary connection | 0.02-0.03 s | 0.02-0.05 s | 0.02-0.04 s (median 0.035 s) |
| a core with no terminal, alone | - | - | 0.00-0.01 s |

The second core costs 0.02-0.04 s of CPU per 30 s with one idle terminal, about 0.1% of one CPU core. The
secondary connection adds nothing measurable to our server. With its last terminal gone, a core costs 0.00-0.01 s
per 30 s.

## What limits each number

- **Gate 1, default: met, by about one noise band.** The run-by-run ratios spread from 1.02x to 1.33x, so only
  medians can be compared. The PTY side drifted up during the hour: its medians were 1.07 s for runs 1-4, 1.00 s
  for runs 5-8, 1.095 s for runs 9-12 and 1.12 s for runs 13-16. Core stayed near 1.27 s, so the ratio fell from
  1.22x after 8 runs to 1.19x after 16. Taken over the four groups of four, the band is 1.12, and 1.19x is inside
  it. D11 defines the band on runs 1-4 and 5-8 only, and by that rule the 16-run median decides. The record shows
  the margin is thin, as in phase 2: 1.21x then, 1.19x now.
- **What sets the server's extra on core.** Our server makes 2377 Mach calls in 20 s on core and about 4 on the
  PTY (phase 2: 2374). These calls come from the hop from the shared reader thread to each pane's parser thread.
  They are the same with and without the `sleep` children, and the count does not change with load. Context
  switches are 11-12% higher on core in both profiles, and BSD calls are the same. The remaining spread follows
  load and core placement, not the build. Our server is niced to 15, and the core server and its holders were not
  checked for nice. Which CPU cores the threads ran on was not measured (as in phase 2). PR E would remove those
  Mach calls.
- **Gate 1, one printer: met with room.** The band is narrow (1.03), and 1.10x is well under 1.3x.
- **Gate 2: met in both profiles. The one-printer reference is weak.** For the default profile, today's 1.945 s is
  below phase 2's median of 2.115 s and inside its range of 1.90-2.52 s, so core has not regressed. For one printer, phase 2 kept only 3 runs, so
  its median of 1.29 s is uncertain to a few hundredths of a second. Today's 1.30 s is 1.01x that median, far
  under 1.2x, which would allow 1.55 s.
- **Load.** The 1-minute load was 4.3-13.4 before the runs. The busiest moments were run 3 core (13.39 after) and
  run 16 core (14.29 after), which came from other sessions. Both values sit inside their side's spread.
- **Second core.** Each counter has the 0.01 s resolution of `ps` over 30 s windows, so 0.02-0.04 s is near the
  floor. The test used one terminal in the legacy core, not 30. A core does per-terminal work only when a
  terminal prints, and the busy profile above measures that cost.

Nothing needs a rerun for the gates. Two reruns would narrow the default margin if a later change moves it: the
default profile in a quiet hour (load below 6), and a check of which CPU cores the reader and parser threads use.

## Files

The artifacts are in herdr-core's state folder on m4m, under `herdr-core-p1/m295/`:

- `driver.sh`: the whole run under the lock.
- `analyze.py`: medians, the noise band and both gates.
- `second_core_idle.sh`
- `run-*.txt` and `top-*.txt`: every run and its counters.
- `summary.md`, `second-core.txt`, `driver.log`
- `herdr-415448a2`: the measured binary.

The first try of `second_core_idle.sh` failed before it measured anything. It read pane labels from `hc ls`,
which lists none. It now reads them from `hc get`, and its rerun under the lock is the one recorded above.
