## 1. Plan

- [x] 1.1 This change, validated with `openspec validate core-status-mirror --strict`

## 2. Pane layer

- [x] 2.1 `CoreStatusRecord` / `CoreStatusWord` / `CoreStatusApp`, mapping and `encode` (verifier 1-3)
- [x] 2.2 `EscapeScanner` and `StatusMirror` gate in `core_backend/status_mirror.rs` (verifier 4-7)
- [x] 2.3 Platform: controlling terminal device of a pid (macOS, Linux; none elsewhere) and the checked write (verifier 8)
- [x] 2.4 Parser thread owns the mirror; one-slot mailbox and wake on `CorePaneIo`; `PaneIo`/`PaneRuntime`/`TerminalRuntime` pass-through

## 3. Server

- [x] 3.1 `[experimental] core_status_mirror` config + test (verifier 9)
- [x] 3.2 Hook in `emit_pane_state_update`; startup and reload apply; sweep on toggle (verifier 10)
- [x] 3.3 Echo test: record bytes leave screen and detection unchanged (verifier 11)

## 4. Live

- [x] 4.1 Live core tests: mirrored `working` in `terminal.get`, then cleared; no hook report (verifier 12-13); V8 unchanged (verifier 14); server path in `scripts/core_live_test.sh` (verifier 15)
- [x] 4.2 Docs: `config-reference.json` entry
- [x] 4.3 `just check` green
- [ ] 4.4 Beta with the flag on in ac's m4m config; ac checks the Inbox (verifier 16)
