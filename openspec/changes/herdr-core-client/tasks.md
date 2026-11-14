## 1. Pilot setup

- [x] 1.1 Read core's install script, then install hc 0.1.17 into `~/.local/bin` on m4m
- [x] 1.2 Use a separate core session (`HC_SESSION=pilot`); never touch the user's default core

## 2. Live questions (design.md § 5)

- [x] 2.1 Q1: Claude Code 2.1.316 and Codex 0.166.0 emit OSC 7501 (idle, working, done seen)
- [ ] 2.2 Q2: a Claude Code hook sees `HC_TERMINAL_ID` and can label its terminal with the session id
- [ ] 2.3 Q3: reboot recovery of kept terminals, saved screens and documents
- [ ] 2.4 Q4: CPU and memory footprint, an idle core with 30 terminals against today's herdr server
- [ ] 2.5 Q5: typing latency and frame cost through a raw attach stream with a client-side VT
- [ ] 2.6 Q6: `hc update` with viewers attached (restart half done: programs keep running)
- [ ] 2.7 Q7: remote dormancy with ac-mbm5 as a machine and the lid closed (through manager-aruba)
- [ ] 2.8 Q8: a launchd-started core survives a macOS logout
- [ ] 2.9 Q9: which process macOS privacy prompts name for an hc terminal, and after `hc update`
- [ ] 2.10 Q10: Codex app-server mode and core's `--no-daemon` rule
- [ ] 2.11 Q11: `hc keys` single characters; text without Enter
- [ ] 2.12 Q12: `hc read` text good enough for our manifests; OSC title as `title`
- [ ] 2.13 Q13: `hc send` holds while a person types through an attach stream
- [ ] 2.14 Q14: pilot core and the herdr server side by side (no interference seen so far)
- [ ] 2.15 Q15: share the `herdr-app` document round trip (needs herdr-app on the pilot session)

## 3. Decision

- [x] 3.1 Bring ac the blockers left after the pilot and a recommended shape: design.md § 6 (Shape B first), decisions on the desk (§ 6.6)
- [ ] 3.2 On ac's choice, add spec deltas and the build plan to this change

## 4. Migration (design.md § 6.4); each phase gets its own OpenSpec change and Gitea issue

- [ ] 4.0 Phase 0: upgrade the pilot core to 0.1.21; answer Q4, Q6, Q8, Q9, Q11, Q12, Q13 (go/no-go)
- [ ] 4.1 Phase 1: runtime seam over `src/pty/` (characterization tests first, roundtable)
- [ ] 4.2 Phase 2: core backend behind `[experimental] runtime = "core"`; identity via labels (Q2); 7501 + detection
- [ ] 4.3 Phase 3: restart re-adopts core terminals; mixed sessions; default on, m4m then both Macs
- [ ] 4.4 Phase 4: layout in a core document (ac's choice § 6.6 item 2; Q15)
- [ ] 4.5 Phase 5: remote via core machines; dormancy (issue 260)
- [ ] 4.6 Phase 6: delete the PTY runtime, live handoff and our remote host
