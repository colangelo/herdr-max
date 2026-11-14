## 1. Pilot setup

- [x] 1.1 Read core's install script, then install hc 0.1.17 into `~/.local/bin` on m4m
- [x] 1.2 Use a separate core session (`HC_SESSION=pilot`); never touch the user's default core

## 2. Live questions (design.md § 5)

- [x] 2.1 Q1: Claude Code 2.1.316 and Codex 0.166.0 emit OSC 7501 (idle, working, done seen)
- [x] 2.2 Q2: a program in a core terminal sees `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET` and `HC_SESSION`,
  and `hc label $HC_TERMINAL_ID k=v` works from inside (2026-11-14, hc 0.1.21); a hook inherits that
  environment
- [ ] 2.3 Q3: reboot recovery of kept terminals, saved screens and documents
- [x] 2.4 Q4: 30 idle shells, core+holder 15.1 MB vs herdr 20.4 MB; busy, 0.31 s vs 0.83 s CPU per 30 s; observe +0.07 s, polling +0.38 s (design.md § 5a)
- [ ] 2.5 Q5: typing latency and frame cost through a raw attach stream with a client-side VT
- [x] 2.6 Q6: restart/update close the stream; reattach in 0.29-0.36 s; programs untouched; `seq` restarts at 0
- [ ] 2.7 Q7: remote dormancy with ac-mbm5 as a machine and the lid closed (through manager-aruba)
- [ ] 2.8 Q8: a launchd-started core survives a macOS logout (holder is in `gui/501`, likely not; needs a real logout, asked on the desk; gate for phase 3)
- [x] 2.9 Q9: the holder (`dev.herdr.core`) is responsible; open terminals keep their holder across `hc update`
- [ ] 2.10 Q10: Codex app-server mode and core's `--no-daemon` rule
- [x] 2.11 Q11: `hc keys` takes named keys and unshifted characters only; stream DATA types any text with no Enter
- [x] 2.12 Q12: `hc read` text equals our detection text; Claude reports 7501 and stops animating the title in core
- [x] 2.13 Q13: send waits for the person (0.8 s after Enter); an unfinished draft holds it 60 s, then it pastes onto the draft
- [ ] 2.14 Q14: pilot core and the herdr server side by side (no interference seen so far)
- [x] 2.15 Q15: not needed: ac chose herdr-max's own layout document (design.md § 6.6)

## 3. Decision

- [x] 3.1 Bring ac the blockers left after the pilot and a recommended shape: design.md § 6 (Shape B first), decisions on the desk (§ 6.6)
- [x] 3.2 ac chose Shape B first and our own layout document (2026-11-14, design.md § 6.6); each
  phase carries its own spec deltas and plan (runtime-seam, core-backend, ...)

## 4. Migration (design.md § 6.4); each phase gets its own OpenSpec change and Gitea issue

- [x] 4.0 Phase 0: pilot core on 0.1.21; Q4, Q6, Q9, Q11, Q12, Q13 green, Q8 open (gate for phase 3): go for phase 1 (design.md § 5a)
- [ ] 4.1 Phase 1: runtime seam over `src/pty/` (characterization tests first, roundtable)
- [ ] 4.2 Phase 2: core backend behind `[experimental] runtime = "core"`; identity via labels (Q2); 7501 + detection
- [ ] 4.3 Phase 3: restart re-adopts core terminals; mixed sessions; default on, m4m then both Macs
- [ ] 4.4 Phase 4: layout in herdr-max's own core document (ac's choice, § 6.6 item 2; Q15 not needed)
- [ ] 4.5 Phase 5: remote via core machines; dormancy (issue 260)
- [ ] 4.6 Phase 6: delete the PTY runtime, live handoff and our remote host
