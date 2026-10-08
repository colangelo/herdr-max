## 1. Repoint manifest URLs to the fork

- [x] 1.1 Point `STABLE_UPDATE_MANIFEST_URL` / `PREVIEW_UPDATE_MANIFEST_URL` in `src/update.rs` at `raw.githubusercontent.com/colangelo/herdr/master/website/{latest,preview}.json`
- [x] 1.2 Point the same two constants in `src/remote/unix.rs` at the fork's raw-GitHub manifests, kept in sync with `src/update.rs`
- [x] 1.3 Update the `src/update.rs` module doc comment to reflect the fork manifest source
- [x] 1.4 Leave `src/detect/manifest_update.rs` `DEFAULT_CATALOG_URL` on upstream `herdr.dev/agent-detection/index.toml` (fork hosts no catalog)
- [x] 1.5 Confirm `[update].version_check` stays defaulted to `true` (now fork-safe once data lands)

## 2. Serve fork release data from the fork's latest.json

- [x] 2.1 Add a fork-flavored publish step to the release flow — a new `update-latest-json` job in `.github/workflows/release.yml` runs after `release`, rewriting `website/latest.json` with fork release data on each stable `-ac` release: the base version the binary reports (`X.Y.Z`), fork protocol, and assets pointing at `github.com/colangelo/herdr/releases/download/vX.Y.Z-ac/...`
- [x] 2.2 Commit the regenerated `website/latest.json` back to `master` so the raw manifest is current (fork-owned equivalent of the removed upstream `update-latest-json` job, using the default `GITHUB_TOKEN`)
- [x] 2.3 Seed `website/latest.json` with correct fork data now (regenerated from the real `v0.7.1-ac` release: base version `0.7.1`, fork protocol 15, `colangelo/herdr` assets, fork release notes, clean release archive) so no fork user is pointed at upstream binaries in the interim
- [x] 2.4 Make `website/preview.json` fork-correct — pass `--repo ${{ github.repository }}` to `preview.py notes`/`manifest` in `.github/workflows/preview.yml` so preview asset URLs resolve to `colangelo/herdr` (the committed `preview.json` regenerates with fork URLs on the next preview run)
- [x] 2.5 Add `--tag` (read a fork-suffixed release tag) and `--force` (skip the monotonic-version guard for `-ac.N` hotfixes / interim corrections) to `scripts/changelog.py sync-latest-json`, with unit tests, keeping the upstream path unchanged

## 3. Verification

- [x] 3.1 `just check` green (no test pins the manifest URL; `test_changelog` covers the new `--tag`/`--force` paths)
- [x] 3.2 Manually fetch both raw URLs and confirm they return the fork's manifests — stable `latest.json` serves fork data (version 0.7.1, `colangelo/herdr` assets); preview `preview.json` still holds the pre-fix upstream snapshot and regenerates to `colangelo/herdr` URLs on the next preview run (workflow fixed in 2.4)
- [x] 3.3 Dogfood: on a fork stable build, run `herdr update` / a background version check and confirm it compares against fork data and offers a fork binary (not upstream) — verified 2026-07-18 via the real `check_latest()` path on a source build (0.7.4): it fetches the fork stable manifest (0.7.1) and the resolved `macos-aarch64` asset is `github.com/colangelo/herdr/releases/download/v0.7.1-ac/herdr-macos-aarch64`, not upstream
- [x] 3.4 Confirm the stable/preview channel selection still routes to the correct manifest after the repoint — verified in code: `UpdateChannel::configured()` selects `STABLE`/`PREVIEW_UPDATE_MANIFEST_URL`
- [x] 3.5 On the next stable `-ac` release, confirm the `update-latest-json` job publishes fork data to `website/latest.json` — verified at `v0.7.4-ac` (2026-07-18): the release run's `update-latest-json` job succeeded and committed `docs: update fork latest manifest for v0.7.4-ac` to `master`; the served manifest (`raw.githubusercontent.com/colangelo/herdr/master/website/latest.json`) now reports version `0.7.4`, protocol `18`, and all four assets resolve to `github.com/colangelo/herdr/releases/download/v0.7.4-ac/...` (fork binaries, not upstream)

## Notes

- Sections 1 and 2 are implemented in this change. `website/latest.json` carries fork release data, so the repointed stable manifest is fork-correct; the release job keeps it current on every `-ac` release. All verification tasks are confirmed: 3.2–3.4 on 2026-07-18 (stable manifest + dogfooded update path resolve fork data/binaries; channel routing verified in code), and 3.5 at the `v0.7.4-ac` release (2026-07-18) — the `update-latest-json` job published fork data (version `0.7.4`, protocol `18`, `colangelo/herdr` assets) to `master`. Change complete and ready to archive.
- The `--force` flag on the release job supports fork base-version reuse across `-ac.N` hotfixes (e.g. `0.7.4-ac` then `0.7.4-ac.2`), which the monotonic guard would otherwise reject.
