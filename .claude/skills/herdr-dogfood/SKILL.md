---
name: herdr-dogfood
description: Use when a change to the herdr fork should end up visible in the user's RUNNING herdr — "ship it so I can see it", "build a beta and test it live", "turn the new option on" — or when adding a config option that must apply via reload without restarting the server.
---

# Dogfooding a herdr feature (fork → beta → live install)

The loop (since the 0.9.3 cutover, October 2026): branch off `sync/merge` →
implement → every cargo run through the lead's runner → commit → draft-PR CI →
fast-forward `master` → `just beta` (beta.yml from master) → install on m4m by
mac-m4, then on mbm5 by mac-m5 via manager-aruba (`just brew-upgrade herdr-beta`,
live handoff, panes preserved) → enable in config → `herdr-beta server
reload-config` → user verifies. Never `git push origin master` new work directly.

**REQUIRED BACKGROUND:** `.claude/skills/herdr-release/SKILL.md` (Beta channel
+ live handoff sections) for beta mechanics, handoff preconditions, and failure
recovery. This skill is the feature-side recipe that ends there.

## New config option: wiring checklist

Follow the `workspace_number_color` pattern end to end (grep it for a worked
example). First classify the option (AGENTS.md "Runtime/client boundary"): since
0.9.3 the TUI is a client, so anything that only changes how things are DRAWN
(sidebar, tabs, colours, spinners, motion) is read by the client shell, and only
runtime facts go through the server's `AppState`. A `[ui]` option touches SIX places:

1. `src/config/model.rs` — `UiConfig` field + `Default` impl; enums modeled on
   `WorkspaceSortConfig` (`#[serde(rename_all = "lowercase")]`); extend the
   `[ui]` parse test.
2. `src/config.rs` — re-export any new type.
3. The state that holds it: for presentation, a `ClientShellConfig` field in
   `src/client/shell/state.rs`; for a runtime fact, an `AppState` field in
   `src/app/state.rs` + `test_new()` default (+ a resolution helper if the
   option has fallback semantics).
4. **BOTH** wiring points of that state: for the client,
   `ClientShellConfig::from_config` AND `apply_live_config` in
   `src/client/shell/config.rs`; for the server, startup construction AND
   `apply_live_config` in `src/app/mod.rs`. Missing the live-reload one means the
   feature stays invisible after `reload-config`; only a restart would show it.
5. `src/main.rs` — commented entry in the generated config template
   (verify later with `herdr-beta --default-config`).
6. Docs: `docs/next/website/src/content/docs/configuration.mdx` +
   `docs/next/CHANGELOG.md` (amend the unreleased entry if the same feature
   family already has one). Never stable docs / root README / root CHANGELOG.

No `Cargo.toml` version bump (stays base `X.Y.Z`; the beta workflow stamps the
full version), no protocol bump for TUI-only changes.

## Ship recipe

Work in your own worktree on a branch off `sync/merge` (`wt switch --create
fix/<slug> --base internal/sync/merge`), never on `master` and never in a tree
another agent holds.

```bash
# 1. Build and test ONLY through the lead's runner (shared CPU lock, 6 jobs):
#    append `NEEDS-RUN: <command>` to <worktree>/.local/port/PROGRESS.md and read
#    <worktree>/.local/port/runs/RESULTS.md. Order: cargo check --all-targets --locked,
#    targeted nextest, cargo clippy --all-targets --locked -- -D warnings, just check.
#    Never run cargo directly; release-profile runs are lead-only.
git status --short                # stage ONLY your files, never `git add -A` blind
git add <files> && git commit     # lowercase conventional; state the message, no
                                  # co-author lines; refs <full Gitea issue URL>
git push internal <branch>        # only after the runner's `just check` is green

# 2. CI: a draft PR on GitHub, for CI only (nobody reviews or merges it there).
git push origin <branch>
command gh pr create --repo colangelo/herdr-max --draft --base master --head <branch> \
  --title "<subject>" --body "refs <full Gitea issue URL>"
command gh pr checks --repo colangelo/herdr-max --watch

# 3. Adopt: the lead (or `herdr`) fast-forwards master, never a direct push of new work.
git merge --ff-only <branch> && git push internal master && git push origin master

# 4. Beta from master, then the installs.
just beta                         # dispatches beta.yml from origin/master
command gh run watch <run-id> --repo colangelo/herdr-max --exit-status
```

5. Install, seamless (live handoff, panes and sessions kept): mac-m4 runs
   `just brew-upgrade herdr-beta` on m4m; then ask manager-aruba to have mac-m5
   do the same on mbm5. Each reports `herdr-beta --version`
   (`X.Y.Z-ac-beta.<run>-<codename>` = the new build).

If `gh` or the push to `origin` says the token is invalid, ask mac-m4 to log in
again on m4m; the Gitea steps keep working meanwhile.

If master CI is red for unrelated reasons, fix or backlog before shipping —
never dismiss as "pre-existing".

## Show the feature

1. Edit `~/.config/herdr/config.toml` (usually under `[ui]`).
2. `herdr-beta server reload-config` — expect `"status":"applied"` with empty
   diagnostics. Always run it; it is idempotent — don't burn time inferring
   whether it's needed. Use the binary matching the running channel
   (`herdr-beta` vs `herdr`).
3. Tell the user exactly what to look at. Stale visuals repaint on the next
   focus change.

## Pitfalls

| Symptom | Cause |
|---|---|
| Option set in config but nothing changes after reload | wiring point 4 incomplete (`apply_live_config` missing) |
| `--default-config` doesn't list the option | `src/main.rs` template entry missing (point 5) |
| Beta built without the change | master not fast-forwarded and pushed to `origin` before `just beta` |
| `cargo` fights other agents for CPU / a load alert | ran cargo directly instead of through the runner |
| `brew upgrade` says already up to date | `brew update` missing or the formula job hasn't finished |
| Unrelated files swept into the commit | `git add -A` with sibling-agent WIP present |
