## 1. Server

- [x] 1.1 `PaneMarkSeenParams`, `Method::PaneMarkSeen` (`pane.mark_seen`, client-shell, UI-changing), `ResponseResult::PaneMarkSeen { changed }`
- [x] 1.2 `AppState::mark_panes_seen(scope) -> usize` + unit tests (verifier 1-3)
- [x] 1.3 API handler, publish only on change, tests (verifier 4-6)
- [x] 1.4 schema / method list tests (verifier 7)

## 2. CLI

- [x] 2.1 `herdr pane mark-seen [--workspace <id>]` + test (verifier 8)

## 3. Client

- [x] 3.1 menu actions, unread flags on targets, method-support gating
- [x] 3.2 rendered-layout and dispatch tests (verifier 9-12)

## 4. Docs and ship

- [x] 4.1 docs/next CLI + socket API pages
- [ ] 4.2 `just check` green, commits, merge to master, beta-local, install both Macs (verifier 13)
