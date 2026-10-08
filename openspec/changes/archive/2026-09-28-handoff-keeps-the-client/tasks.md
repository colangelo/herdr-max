## 1. Carried size

- [x] 1.1 Manifest `client_size`; export `effective_size` at handoff; import sets the no-client size; tests

## 2. Reconnect

- [x] 2.1 Shared handoff reason; the app client waits for the new server and reattaches; tests

## 3. Ship

- [x] 3.1 `just check` green
- [x] 3.2 Beta; harness proof (a client in a throwaway pane survives a live handoff at full size)

Verified 2026-09-28 on m4m with the `0.8.2-ac-beta.101-kulusevski` release binary in a throwaway client-in-a-pane harness: with beta.101 on both sides the client reconnected and panes kept 56×70 through a live handoff; see the issue for the mixed-version cases.
