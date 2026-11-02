## Why

Upstream split herdr into **herdr-core** (`hc`, a headless terminal runtime with an OpenRPC API) and
**herdr-app** (a desktop app built on core). The TUI that herdr-max forks keeps working, but upstream
work has moved: TUI commits fell from about 40 a week in late September to 5 in the week of
2026-10-26, while core and the app took 172 commits in the same week. Core invites other clients
("a phone app, an editor panel, a Stream Deck", herdr.dev/core).

ac (2026-11-02) chose to stay on the TUI fork for now, run a core pilot on m4m, and find what is
missing for herdr-max's **full** functionality, in both directions. The aim for the long run is for
herdr-max to become one client of herdr-core, so the cut from upstream is cleaner.

Record: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/267

## What Changes

Nothing ships yet. This change holds the exploration and the decision it leads to:

- `design.md`: the model mapping, the gap table (what herdr-max needs and core lacks), what core has
  that herdr-max lacks, mods against our plugins, where the layout should live, the fleet-tooling
  verbs, the ranked blockers, and the questions only a live pilot can settle, with the answers so far.
- `tasks.md`: the remaining pilot questions and the decision gate.

After the decision, this change gets the spec deltas and the build plan for the chosen shape
(direct client, or an adapter that replaces the server's PTY layer with core terminals).

## Impact

- A pilot core on m4m only (`~/.local/bin/hc`, `HC_SESSION=pilot`), apart from the working herdr
  session. No change to the running herdr, the betas or ac-mbm5.
- Later, depending on the shape: the PTY layer (`src/pty/`), the client protocol, detection, and the
  fleet tools that drive the herdr CLI (agent-bell, herdr-compact, session-status, shepherd, gestore).
