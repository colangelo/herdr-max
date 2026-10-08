## Why

Herdr's self-update check calls home to upstream's hosted manifests at `herdr.dev/latest.json` and `herdr.dev/preview.json`. On this fork (`colangelo/herdr`), that means a fork user's `version_check` and `herdr update` compare against — and would install — **upstream** `ogulcancelik/herdr` binaries, silently replacing the fork. The fork controls neither `herdr.dev` nor upstream's release assets, so the only way to make the update check fork-correct is to point it at manifests the fork owns.

The fork already publishes its own binaries via GitHub Releases and a Homebrew tap, and keeps `website/latest.json` / `website/preview.json` checked into the repo. Those files can be served directly over raw GitHub, giving the fork full control of what "an update is available" means for its own users — without depending on any hosted infrastructure.

## What Changes

- Repoint the stable and preview update-manifest URLs away from upstream `herdr.dev` to the fork's own checked-in manifests, served via raw GitHub:
  - stable → `https://raw.githubusercontent.com/colangelo/herdr/master/website/latest.json`
  - preview → `https://raw.githubusercontent.com/colangelo/herdr/master/website/preview.json`
- Keep the agent-detection manifest **catalog** URL on upstream `herdr.dev/agent-detection/index.toml`. The fork does not host a manifest catalog, and agent-detection manifests are a separate concern from release self-update.
- Populate the fork's `website/latest.json` with **fork** release data (fork version, fork protocol, assets pointing at `github.com/colangelo/herdr/releases/...`) as part of the fork's `release-ac` flow, so the repointed stable manifest serves fork data rather than the stale upstream snapshot currently committed.
- Keep `[update].version_check` defaulting to `true`: once the manifests point at the fork and carry fork data, background checks are safe and fork-correct.
- Non-goals: hosting a fork website or CDN, changing the manifest JSON schema, changing the agent-detection catalog format, or altering upstream's release infrastructure.

## Capabilities

### New Capabilities

- `fork-aware-update-check`: The self-update and remote-update paths fetch release manifests from the fork's own repository (raw GitHub) instead of upstream's hosted site, so background version checks and `herdr update` compare against and install fork builds. Covers the stable/preview manifest URLs, the retained upstream agent-detection catalog, and the requirement that the fork's `latest.json` carry fork release data.

### Modified Capabilities

<!-- None. There are no pre-existing OpenSpec specs for the update path; the manifest JSON schema and protocol are unchanged. -->

## Impact

- **Update client** (`src/update.rs`): the `STABLE_UPDATE_MANIFEST_URL` / `PREVIEW_UPDATE_MANIFEST_URL` constants now point at the fork's raw-GitHub manifests.
- **Remote update path** (`src/remote/unix.rs`): the same two constants, kept in sync so `herdr --remote` update checks are also fork-aware.
- **Agent detection** (`src/detect/manifest_update.rs`): unchanged — `DEFAULT_CATALOG_URL` stays on upstream `herdr.dev/agent-detection`.
- **Release pipeline** (`.github/workflows/release.yml` / `justfile` `release-ac`): a fork-flavored publish step writes fork release data into `website/latest.json` on the repo's default branch so the raw manifest is current. This re-introduces a fork-owned equivalent of the `update-latest-json` job that the fork previously removed (it removed the *upstream* job, which needed upstream deploy keys and served upstream data).
- **Runtime/protocol**: no new socket message, no protocol change, no server state. The manifest JSON schema is unchanged.
- **Boundary guardrail**: the manifest URL is shared runtime/distribution behavior, named neutrally (update manifest, not a UI surface); it does not deepen server/TUI coupling.
