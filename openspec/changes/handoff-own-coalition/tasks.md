# Tasks (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/288)

- [ ] 1.1 `spawn_handoff_server` (macOS): launchd job first, disclaimed spawn as fallback; returns 0 for launchd (D1)
- [ ] 1.2 handoff takes the replacement pid from the socket peer when the spawn returned 0 (D2)
- [ ] 1.3 rollback for a non-child replacement: guarded SIGKILL, no wait; pid 0 warns (D3)
- [ ] 2.1 unit tests V4-V6
- [ ] 2.2 live V1-V3 on a throwaway herdr (never ac's session), `just check`, `just core-live-test`
- [ ] 3.1 beta, then ac's server moves on the next install (its panes adopted by fd keep their coalition until respawned)
