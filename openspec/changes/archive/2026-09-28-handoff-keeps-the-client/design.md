## Findings

`disconnect_all_clients_for_handoff` drops clients but does not resize PTYs,
because `resize_shared_runtime_to_effective_size` needs a foreground client. The
shrink happens in the importing server: `render_and_stream` with no render
targets calls `render_virtual_with_runtime_registry` at `effective_size` with
`resize_panes = pane_infos.is_empty()`. Its first frame after import therefore
resizes every pane to `headless_size`. This was traced with a backtrace on
`PaneRuntime::resize` in a throwaway harness.

## Decisions

**Carried size.** The manifest gains `client_size: Option<(u16, u16)>`, taken
from `effective_size` before clients are dropped. The importing server keeps it
as `handoff_client_size`; `detached_size()` returns it, or `headless_size` when
absent, and replaces `headless_size` wherever the no-client size is chosen. The
first attach clears it, so a later ordinary detach behaves as today. A handoff
with no client attached exports the carried size again, so a chain of handoffs
keeps it.

**Reconnect.** The handoff shutdown reason becomes a shared constant. The app
client (`run_client`) treats `ServerShutdown` with that exact reason as "handed
off": it restores the terminal as on any exit, prints one line, polls the client
socket until it connects (30 s cap), then runs the client again. Reattaching is
a normal handshake at the terminal's current size. Matching the unchanged text
keeps the wire format and protocol version intact and works against older
servers.
