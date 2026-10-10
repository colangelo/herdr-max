# herdr task runner
set windows-shell := ["cmd.exe", "/d", "/s", "/c"]

python := if os() == "windows" { "python" } else { "python3" }

# The vendored libghostty-vt needs zig 0.16. Use ZIG when set, else the
# keg-only zig@0.16 when it is installed (the global zig can be newer and then
# fails the build), else whatever `zig` is on PATH.
export ZIG := env_var_or_default("ZIG", if path_exists("/opt/homebrew/opt/zig@0.16/bin/zig") == "true" { "/opt/homebrew/opt/zig@0.16/bin/zig" } else { "zig" })

# List available recipes (default)
default:
    @just --list

# Run tests
test:
    cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never
    just maintenance-test
    just ui-hot-path-architecture-test
    just integration-assets-test
    just docs-contract-test

# Run repository maintenance contract tests
maintenance-test:
    {{python}} -m unittest scripts.test_agent_detection_manifest_check scripts.test_beta_build_id scripts.test_beta_publish scripts.test_changelog scripts.test_claude_mod_hint scripts.test_config_reference_check scripts.test_conventional_commits scripts.test_hermes_integration_asset scripts.test_package_windows_conpty scripts.test_preview scripts.test_release scripts.test_unix_installer scripts.test_vendor_libghostty_vt scripts.test_vendor_portable_pty scripts.test_windows_cross scripts.test_windows_input
    bun test scripts/release-workflows.test.ts

# Local interactive Windows Terminal input qualification (never runs in normal CI).
[windows]
test-windows-input *args:
    pwsh -NoProfile -File scripts/test_windows_input.ps1 -AllowInputInjection -ClearClipboard {{args}}

# Run one nextest filter, e.g. `just test-one codex_stale_working`
test-one filter:
    cargo nextest run --locked "{{filter}}" --status-level fail --final-status-level fail --failure-output final --success-output never

# Enforce deterministic UI hot-path architecture boundaries
ui-hot-path-architecture-test:
    {{python}} -m unittest scripts.test_ui_hot_path_architecture

# Run fast local lint checks
[unix]
lint:
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings

[script("powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File")]
[windows]
lint:
    & .\scripts\windows_check.ps1 -Mode lint

# Run PR CI checks
ci filter='all()': lint
    just ci-tests "{{filter}}"

# Keep the test build independently configurable from clippy in CI.
ci-tests filter='all()':
    cargo nextest run --locked -E "{{filter}}" --status-level fail --final-status-level slow --failure-output final --success-output never
    just maintenance-test
    just ui-hot-path-architecture-test
    just integration-assets-test

# Download the Windows SDK once (requires xwin; prompts for Microsoft's SDK license)
[unix]
setup-windows-cross *args:
    {{python}} scripts/windows_cross.py setup {{args}}

# Run Windows target lint from Unix/macOS to catch cfg(windows) compile and clippy failures before CI
[unix]
windows-lint:
    {{python}} scripts/windows_cross.py lint

# Check formatting + run unit tests + Windows target lint + documentation contract tests
[unix]
check: ci windows-lint
    just docs-contract-test
    @echo "docs reminder: if this changes user-facing behavior, make sure the relevant release docs are updated or called out before release."

[script("powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File")]
[windows]
check:
    & .\scripts\windows_check.ps1 -Mode check

# Install repo-local git hooks
install-hooks:
    git config core.hooksPath .githooks
    chmod +x .githooks/pre-commit
    chmod +x .githooks/commit-msg
    @echo "installed git hooks from .githooks"

# Build release binary
[unix]
build:
    cargo build --release --locked

[windows]
build:
    python scripts/package_windows_conpty.py build-local

# Non-gating full-render scaling profile for background workspaces and active panes
bench-render-scale:
    cargo test --release --locked --bin herdr render_scale_profile -- --ignored --nocapture --test-threads=1

# Profile terminal target name resolution at increasing pane counts.
bench-terminal-targets:
    cargo test --release --locked --bin herdr terminal_target_lookup_profile -- --ignored --nocapture --test-threads=1

# Profile Windows foreground inspection of isolated idle shells, without a server.
[windows]
bench-process-inspection:
    cargo test --release --locked --bin herdr windows_process_inspection_profile -- --ignored --nocapture --test-threads=1

# Profile BSP split collection and construction with balanced and skewed trees.
bench-bsp-layout:
    cargo test --release --locked --bin herdr bsp_layout_profile -- --ignored --nocapture --test-threads=1

# Profile full and retained text, static-image, and unchanged-image updates.
bench-retained-graphics:
    cargo test --release --locked --bin herdr render_scale_profile_retained_graphics -- --ignored --nocapture --test-threads=1

# Profile first-batch latency and aggregate drain cost for external API bursts.
bench-api-fairness:
    cargo test --release --locked --bin herdr external_api_burst_profile -- --ignored --nocapture --test-threads=1

# ~3-5 minute CPU comparison; downloads stable unless HERDR_PERF_BASELINE_BIN is set
bench-release-smoke:
    cargo build --release --locked
    scripts/release_perf_smoke.sh "${CARGO_TARGET_DIR:-target}/release/herdr"

# Test public documentation snapshot and release lifecycle tooling
docs-contract-test:
    bun test ./scripts/docs

# Test bundled agent integration assets
integration-assets-test:
    bun test src/integration/assets/herdr-agent-state.test.ts
    bun test src/integration/assets/opencode/herdr-agent-state.test.ts
    bun test src/integration/assets/opencode/herdr-tui-session.test.ts

# Regenerate the C API bindings with bindgen-cli 0.72.1
libghostty-bindings *clang_args:
    bash scripts/generate_libghostty_bindings.sh {{clang_args}}

# Build the vendored libghostty-vt source dist
build-libghostty-vt:
    scripts/build_vendored_libghostty_vt.sh

# Check that release docs and changelog have been finalized from docs/next before release
release-docs-check:
    python3 scripts/agent_detection_manifest_check.py --require-all-published
    python3 scripts/config_reference_check.py
    # fork: `scripts/docs/versions.mjs check` is omitted — it asserts the
    # docs/versions manifest's current version == distribution/latest.json's version,
    # and the fork's latest.json is fork-scoped (its own -ac releases) while
    # docs/versions tracks upstream's herdr.dev release docs. The fork does not
    # publish herdr.dev, so the two are intentionally out of step.
    # `just docs-contract-test` below still tests the snapshot/version tooling.
    node scripts/docs/preview.mjs check
    just docs-contract-test
    @test -f docs/next/README.md
    @test -f docs/next/README.zh-CN.md
    @if ! diff -u CHANGELOG.md docs/next/CHANGELOG.md; then \
        echo "error: CHANGELOG.md differs from docs/next/CHANGELOG.md; finalize release notes before releasing"; \
        exit 1; \
    fi
    @for file in CONFIGURATION.md INTEGRATIONS.md SOCKET_API.md; do \
        if [ -e "$file" ]; then \
            echo "error: $file was replaced by technical docs; remove the root copy"; \
            exit 1; \
        fi; \
    done
    @test -d docs/next/website/src/content/docs
    @for file in docs/next/website/src/content/docs/*.mdx; do \
        for locale in ja zh-cn; do \
            translated="docs/next/website/src/content/docs/$locale/$(basename "$file")"; \
            if [ ! -f "$translated" ]; then \
                echo "error: $translated is missing; translate next docs before releasing"; \
                exit 1; \
            fi; \
        done; \
    done
    @for file in docs/next/website/src/content/docs/ja/*.mdx docs/next/website/src/content/docs/zh-cn/*.mdx; do \
        staged="docs/next/website/src/content/docs/$(basename "$file")"; \
        if [ ! -f "$staged" ]; then \
            echo "error: $file has no matching english doc; remove the stale translation"; \
            exit 1; \
        fi; \
    done

# Validate release docs, render scaling, and end-to-end CPU before release preparation
pre-release-check:
    just release-docs-check
    just bench-render-scale
    just bench-release-smoke
    @echo "release review required: investigate material render-scaling regressions before publishing."
    @echo "release review required: update skills/herdr/SKILL.md for this stable release so it matches the current CLI, IDs, agent lifecycle semantics, and safety guidance."
    @echo "release policy: do not update skills/herdr/SKILL.md between stable releases; preview builds keep the latest stable skill."

# Publish a preview by pushing an admin-owned tag at the selected source commit.
preview $ref='HEAD':
    git fetch --prune origin '+refs/heads/master:refs/remotes/origin/master' '+refs/heads/release/*:refs/remotes/origin/release/*' --tags
    @set -eu; \
    commit="$(python3 scripts/release.py preview-source --commit "$ref")"; \
    day="$(git show -s --format=%cs "$commit")"; \
    short="$(git rev-parse --short=12 "$commit")"; \
    tag="preview-$day-$short"; \
    git tag -a "$tag" "$commit" -m "$tag"; \
    git push origin "refs/tags/$tag"

# In a checkout based on the selected preview, prepare release-only metadata.
release-prepare $version $preview:
    @printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { \
        echo "error: version must look like 0.6.6 without a v prefix"; \
        exit 1; \
    }
    @if ! git diff --quiet -- . ':(exclude)skills/herdr/SKILL.md' || \
        ! git diff --cached --quiet -- . ':(exclude)skills/herdr/SKILL.md' || \
        [ -n "$(git ls-files --others --exclude-standard)" ]; then \
        echo "error: commit all changes except skills/herdr/SKILL.md first"; \
        exit 1; \
    fi
    @git fetch origin master --tags
    @if git rev-parse "v$version" >/dev/null 2>&1; then \
        echo "error: tag v$version already exists"; \
        exit 1; \
    fi
    python3 scripts/release.py check-source --preview "$preview"
    just pre-release-check
    python3 scripts/changelog.py prepare --version "$version"
    cp CHANGELOG.md docs/next/CHANGELOG.md
    sed -i.bak "s/^version = \".*\"/version = \"$version\"/" Cargo.toml && rm -f Cargo.toml.bak
    cargo update -p herdr --offline
    just check
    git add CHANGELOG.md docs/next/CHANGELOG.md Cargo.toml Cargo.lock skills/herdr/SKILL.md
    git diff --cached --quiet || git commit -m "release: v$version"
    python3 scripts/release.py check-source --preview "$preview"
    @echo "v$version release commit prepared. Review it, then run: just release-publish $version $preview"

# Tag a prepared preview-based release; never move master to the release candidate.
release-publish $version $preview:
    @printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { \
        echo "error: version must look like 0.6.6 without a v prefix"; \
        exit 1; \
    }
    @if [ -n "$(git status --porcelain)" ]; then \
        echo "error: working tree must be clean before publishing"; \
        exit 1; \
    fi
    @git fetch origin master --tags
    @if git rev-parse "v$version" >/dev/null 2>&1; then \
        echo "error: tag v$version already exists"; \
        exit 1; \
    fi
    @cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"; \
    if [ "$cargo_version" != "$version" ]; then \
        echo "error: Cargo.toml version $cargo_version does not match $version"; \
        exit 1; \
    fi
    just release-docs-check
    python3 scripts/changelog.py extract --version "$version" --output /tmp/herdr-release-notes-check.md
    rm -f /tmp/herdr-release-notes-check.md
    @previous="$(git show origin/master:distribution/latest.json | python3 -c 'import json,sys; print("v" + json.load(sys.stdin)["version"])')"; \
    python3 scripts/release.py check --preview "$preview" --version "$version" --previous "$previous" && \
    git tag -a "v$version" -m "v$version" -m "Preview: $preview" -m "Previous-Stable: $previous"
    git push origin "v$version"
    @echo "v$version released — GitHub Actions building binaries and updating distribution/latest.json"

# Prepare and promote a published preview, not the latest master.
release $version $preview:
    just release-prepare "$version" "$preview"
    just release-publish "$version" "$preview"

# Print default config
default-config:
    cargo run --release --locked -- --default-config

# Fork: release an -ac suffixed build on the upstream base version (usage: just release-ac 0.7.1-ac or 0.7.1-ac.2).
# Cargo.toml keeps the base X.Y.Z (Version::parse requires it); binaries get the
# -ac suffix via HERDR_BUILD_CHANNEL=ac in the release workflow.
release-ac $version:
    @printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+-ac(\.[0-9]+)?$' || { \
        echo "error: version must look like 0.7.1-ac or 0.7.1-ac.2"; \
        exit 1; \
    }
    @if [ -n "$(git status --porcelain)" ]; then \
        echo "error: commit your changes first"; \
        exit 1; \
    fi
    @git fetch origin master --tags
    @if git rev-parse "v$version" >/dev/null 2>&1; then \
        echo "error: tag v$version already exists"; \
        exit 1; \
    fi
    just release-docs-check
    python3 scripts/changelog.py prepare --version "$version"
    cp CHANGELOG.md docs/next/CHANGELOG.md
    @base="$(printf '%s' "$version" | sed 's/-ac.*//')"; \
    sed -i.bak "s/^version = \".*\"/version = \"$base\"/" Cargo.toml && rm -f Cargo.toml.bak; \
    cargo update -p herdr --offline || cargo update -p herdr
    just check
    git add CHANGELOG.md docs/next/CHANGELOG.md Cargo.toml Cargo.lock
    git diff --cached --quiet || git commit -m "release: v$version"
    git tag -a "v$version" -m "v$version"
    git push origin HEAD:master
    git push origin "v$version"
    @echo "v$version released — CI builds -ac binaries and updates colangelo/homebrew-tap"

# Fork: verify the newest -ac release, distribution/latest.json, the live raw-GitHub
# manifest, and the release asset URLs all agree. Defaults to the newest v*-ac tag;
# pass one explicitly to check an older release.
#
# Run this after `just release` AND after every upstream sync. The release
# workflow's update-latest-json job commits the manifest to master, so the sync's
# force-push of a replayed patch set can drop that commit — which silently pins
# every fork binary's update check to an older version, with no failing job to
# show for it. That is exactly what happened to v0.7.4-ac between 2026-07-18 and
# 2026-07-27 (see AC-forks/herdr#38).
latest-json-check tag="":
    @set -e; \
    TAG="{{tag}}"; \
    if [ -z "$TAG" ]; then TAG="$(git tag --list 'v*-ac' 'v*-ac.*' --sort=-v:refname | head -n1)"; fi; \
    test -n "$TAG" || { echo "error: no v*-ac tag found — run: git fetch origin --tags"; exit 1; }; \
    BASE="$(printf '%s' "${TAG#v}" | sed 's/-ac.*//')"; \
    PROTOCOL="$(git show "$TAG:src/protocol/wire.rs" | sed -n 's/^pub const PROTOCOL_VERSION: u32 = \([0-9]*\);/\1/p')"; \
    test -n "$PROTOCOL" || { echo "error: no PROTOCOL_VERSION in $TAG:src/protocol/wire.rs"; exit 1; }; \
    ENDPOINT="$(git show "$TAG:src/protocol/endpoint.rs" | sed -n 's/^pub const ENDPOINT_PROTOCOL_GENERATION: u32 = \([0-9]*\);/\1/p')"; \
    test -n "$ENDPOINT" || { echo "error: no ENDPOINT_PROTOCOL_GENERATION in $TAG:src/protocol/endpoint.rs"; exit 1; }; \
    echo "checking $TAG (version $BASE, protocol $PROTOCOL, endpoint generation $ENDPOINT)"; \
    python3 scripts/changelog.py verify-release-state \
        --repo colangelo/herdr-max \
        --version "$BASE" \
        --tag "$TAG" \
        --protocol "$PROTOCOL" \
        --endpoint-generation "$ENDPOINT" \
        --live-url https://raw.githubusercontent.com/colangelo/herdr-max/master/distribution/latest.json

# Fork: trigger a rolling -ac-beta build from a branch (default master).
# Runs .github/workflows/beta.yml: builds macOS binaries, replaces the rolling
# `beta` prerelease, and updates the colangelo/homebrew-tap herdr-beta formula.
# Install/upgrade the result with `brew install colangelo/tap/herdr-beta`.
# Pass a codename to pin the build's suffix, e.g. `just beta master pirlo`;
# it must be one of the names in beta.yml's pool. Empty derives it from the run.
beta ref="master" codename="":
    command gh workflow run beta.yml --repo colangelo/herdr-max --ref {{ref}} -f ref={{ref}} -f codename={{codename}}
    @echo "beta build dispatched from {{ref}} — watch: gh run watch --repo colangelo/herdr-max"

# Upgrade a Homebrew-installed herdr and live-hand-off the running server onto the
# new binary so panes survive — replicates `herdr update --handoff` for brew installs
# (self-update is disabled for brew, but `server live-handoff` is source-agnostic).
# Usage: just brew-upgrade            # stable formula/binary `herdr`
#        just brew-upgrade herdr-beta # beta formula/binary `herdr-beta`
# Always addresses the brew binary by its full path: another `{{formula}}` earlier
# on PATH (e.g. a hand-built ~/.local/bin/herdr) would otherwise shadow it and the
# upgrade would silently act on the wrong binary.
brew-upgrade formula="herdr":
    @test -x "$(brew --prefix)/bin/{{formula}}" || { echo "$(brew --prefix)/bin/{{formula}} not installed — run: brew install colangelo/tap/{{formula}}"; exit 1; }
    brew update
    brew upgrade {{formula}}
    "$(brew --prefix)/bin/{{formula}}" server live-handoff --import-exe "$(brew --prefix)/bin/{{formula}}"
    @echo "{{formula}} upgraded; running server handed off onto $(brew --prefix)/bin/{{formula}}, panes preserved"

# Hands the current (stable) server off to the herdr-beta binary via live handoff
# (source-agnostic); they share one session socket, so it takes over in place.
# Live-switch the running server to the BETA channel, panes preserved (reattach: herdr-beta)
switch-beta:
    @command -v herdr-beta >/dev/null || { echo "herdr-beta not installed — run: brew install colangelo/tap/herdr-beta"; exit 1; }
    @echo "handing off onto $(command -v herdr-beta)"
    herdr server live-handoff --import-exe "$(command -v herdr-beta)"
    @echo "server is now herdr-beta — reattach with: herdr-beta"

# Hands the current (beta) server off to the herdr binary via live handoff.
# Live-switch the running server to the STABLE channel, panes preserved (reattach: herdr)
switch-stable:
    @command -v herdr >/dev/null || { echo "herdr not installed — run: brew install colangelo/tap/herdr"; exit 1; }
    @echo "handing off onto $(command -v herdr)"
    herdr-beta server live-handoff --import-exe "$(command -v herdr)"
    @echo "server is now herdr — reattach with: herdr"
