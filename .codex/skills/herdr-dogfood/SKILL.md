---
name: herdr-dogfood
description: Use when a change to the herdr fork should end up visible in the user's RUNNING herdr — "ship it so I can see it", "build a beta and test it live", "turn the new option on" — or when adding a config option that must apply via reload without restarting the server.
---

# Dogfooding a herdr feature (fork → beta → live install)

The loop: implement → `just check` → commit (propose message first) → push →
`just beta` → `just brew-upgrade herdr-beta` (live handoff, panes preserved) →
enable in config → `herdr-beta server reload-config` → user verifies.

**REQUIRED BACKGROUND:** `.codex/skills/herdr-release/SKILL.md` (Beta channel
+ live handoff sections) for beta mechanics, handoff preconditions, and failure
recovery. This skill is the feature-side recipe that ends there.

## New config option: wiring checklist

Follow the `workspace_number_color` pattern end to end (grep it for a worked
example). A `[ui]` option touches SIX places:

1. `src/config/model.rs` — `UiConfig` field + `Default` impl; enums modeled on
   `WorkspaceSortConfig` (`#[serde(rename_all = "lowercase")]`); extend the
   `[ui]` parse test.
2. `src/config.rs` — re-export any new type.
3. `src/app/state.rs` — matching `AppState` field + `test_new()` default
   (+ a resolution helper if the option has fallback semantics).
4. `src/app/mod.rs` — **BOTH** wiring points: startup construction AND the
   live-reload path (`apply_live_config`). Missing the second means the
   feature stays invisible after `reload-config`; only a server restart would
   show it.
5. `src/main.rs` — commented entry in the generated config template
   (verify later with `herdr-beta --default-config`).
6. Docs: `docs/next/website/src/content/docs/configuration.mdx` +
   `docs/next/CHANGELOG.md` (amend the unreleased entry if the same feature
   family already has one). Never stable docs / root README / root CHANGELOG.

No `Cargo.toml` version bump (stays base `X.Y.Z`; the beta workflow stamps the
full version), no protocol bump for TUI-only changes.

## Ship recipe

```bash
just check                        # must be green; fix failures, don't bypass
git status --short                # unrelated WIP from sibling agents is common —
                                  # stage ONLY your files, never `git add -A` blind
# propose the commit message and get alignment (lowercase conventional commit)
git add <files> && git commit
git push origin master && git push internal master
just beta                         # dispatches beta.yml from pushed origin/master
command gh run watch <run-id> --repo colangelo/herdr-max --exit-status
just brew-upgrade herdr-beta      # brew upgrade + live handoff; panes preserved
herdr-beta --version              # X.Y.Z-ac-beta.<run>-<codename> = the new build
```

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
| Beta built without the change | commit not pushed to `origin/master` before `just beta` |
| `brew upgrade` says already up to date | `brew update` missing or the formula job hasn't finished |
| Unrelated files swept into the commit | `git add -A` with sibling-agent WIP present |
