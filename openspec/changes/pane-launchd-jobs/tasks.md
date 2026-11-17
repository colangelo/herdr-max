# Tasks (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/288)

- [x] 1.1 keeper `__pane-job` (D1) with its socket protocol, reusing the anchor codec and fd passing
- [x] 1.2 server `JobChild` backend and spawn path with fallback (D2); handoff carries job panes
- [x] 1.3 stale job cleanup for `dev.herdr.pane.` labels
- [x] 1.4 `[experimental] pane_jobs` gate, off by default (V11); keeper disclaims the program (V10); peer-uid check (V13)
- [ ] 2.1 Suspended pane state, kept in layout and session.json, respawn and resume (D3)
- [ ] 2.2 test that the suspended state crosses the stable endpoint contract as optional data with an `Unknown`
  fallback, frozen generation-1 fixtures unchanged (V14)
- [ ] 3.1 tests V3-V6, V8, V10, V11, V13 (fake keeper where possible); live V1-V5, V10 on a throwaway herdr
  (phase 1 done 2026-11-17: unit V5 re-report, V8, V10, V11, V13 and the M2 lost-keeper case; live V1-V5, V10-V12.
  V6 waits for 2.1: today a removed job closes its pane)
- [x] 3.3 live V12 once: handoff to an older beta keeps job panes
- [ ] 3.2 V9 measurements
- [ ] 4.1 beta to both Macs; V7 with one real logout on m4m (ask via the desk first)
- [ ] 4.2 request the same model from herdr-core upstream for core terminals (#287)
