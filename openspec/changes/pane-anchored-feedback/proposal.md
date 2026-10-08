## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/129, asked by ac
2026-09-29 ("would it be a problem to show it in the center of the pane where i
copied the text from?"); scope set in comment 17422 ("your pick, included"):
clipboard feedback from both sources and the pane-action notes, in the pane
they are about, off by default.

Today the "copied to clipboard" box sits at a fixed screen spot
(`[ui.toast.clipboard] position`, default `bottom-center`), and the notes a
pane action leaves ("pane move unavailable", "clear scrollback failed", ...)
sit in the `[ui.toast.herdr]` corner, far from the pane they are about.

## What Changes

- `[ui.toast.clipboard] position = "pane"`: the clipboard box is centered in
  the pane the text came from. Both sources carry that pane: herdr's own
  copies (mouse selection, double-click, copy mode) and an app writing the
  clipboard with OSC 52 (`AppEvent::ClipboardWrite` gains the source pane).
- `[ui.toast.herdr] pane_feedback = "corner" | "pane"` (default `corner`):
  with `pane`, the notes set through `set_pane_move_feedback` are centered in
  the pane that was acted on (the focused pane at that moment).
- Fallback to today's placement when there is no source pane (an API copy),
  or it is gone, not shown in the visible tab, or too small for the box.
- Defaults unchanged. Agent notifications (finished / needs attention for
  other panes) stay in the corner: they are about panes elsewhere.

## Capabilities

### New Capabilities

- `pane-anchored-feedback`: feedback about one pane can be drawn in that pane.

## Impact

- `src/events.rs`: `ClipboardWrite { content, source_pane }`; producers in
  `src/pane.rs` (OSC 52, the pane's id) and `src/app/input/clipboard.rs`
  (herdr copies, from the copy request).
- `src/app/state.rs`: `CopyFeedback.source_pane`,
  `ToastNotification.anchor_pane`, the copy request carries its pane.
- `src/config/model.rs`: `ToastClipboardPosition::Pane`,
  `HerdrToastConfig.pane_feedback`; startup and reload already copy the whole
  `ui.toast` table. Default-config template (`src/main.rs`), the config
  reference (`docs/next/website/src/data/config-reference.json`) and
  `configuration.mdx`.
- `src/ui/status.rs` / `src/ui.rs`: placement, reading `view.pane_infos`
  (already computed); one lookup per shown feedback box, no per-pane work.
- TUI presentation only: no API, protocol or persisted state change.
