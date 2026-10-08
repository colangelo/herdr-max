## 1. The context variant

- [x] 1.1 Add `TerminalInputContext::AppScroll` (`src/app/mod.rs`) with a
      comment recording why it is distinct from `Copy`
- [x] 1.2 Return it from `terminal_input_context()` while `Mode::AppScroll` is
      active; leave every other arm untouched
- [x] 1.3 Confirm `routes_to_terminal()` stays false for it, so repeats
      re-dispatch through the app-level handler

## 2. Tests

- [x] 2.1 Unit: the passthrough mode yields a context, it differs from `Copy`,
      it does not route to the terminal, and a modal mode still yields `None`
- [x] 2.2 End-to-end: a press plus two repeats on a live alt-screen pane send
      three `PageUp` encodings, guarding the exact regression
- [x] 2.3 `just check` green
