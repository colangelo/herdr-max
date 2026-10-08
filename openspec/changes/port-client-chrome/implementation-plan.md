# Implementation sequence

Use tasks.md order for feature commits. For each topic: inspect original fork SHA/tree and spec; recover its pure tests or adapt snapshot/surface harness; implement through upstream client idioms; format only owned modules; run ZIG 0.16 cargo check --bin herdr and feature tests when base permits; update topic progress and only checked tasks; commit with Fork-Topic/Fork-Change/Upstream/issue trailers. Runtime projection lands before dependent pin/todo/sync/log consumers, even when locally authored alongside them. No source edit in another worktree.

Risk classification: release-risk because shared snapshot projection, identities, input and render interact. Characterization oracles are the named fork tests in research port-notes/chrome.md and newer upstream endpoint/completion/graphics tests. Roundtable findings are in design.md. Preserve server resource authority, baseline fields/method shapes and byte codecs.

Deferred base-test failures belong to lead until test-clean tag; author owned tests now and record not run instead of marking them green. Rebase only when source is committed/clean; do not cherry-pick unrelated lead implementation. Once check is green, broaden verification only for explicit parity/performance concerns.
