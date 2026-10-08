# fork-aware-update-check Specification

## Purpose
Ensure the fork's self-update and remote-update paths fetch release manifests from
the fork's own repository (raw GitHub) instead of upstream's hosted site, so
background version checks and `herdr update` compare against and install fork
builds, and the fork's `latest.json` carries fork release data.

## Requirements
### Requirement: Release update checks resolve to the fork's own manifests

The local self-update check (`herdr update` and background `version_check`) and the remote binary-provisioning check (`herdr --remote`) SHALL fetch their release manifests from the fork's own repository over raw GitHub, not from upstream's hosted site. The stable channel SHALL fetch `https://raw.githubusercontent.com/colangelo/herdr/master/website/latest.json` and the preview channel SHALL fetch `https://raw.githubusercontent.com/colangelo/herdr/master/website/preview.json`. The two update paths (`src/update.rs` and `src/remote/unix.rs`) SHALL use the same URLs.

This change SHALL NOT alter the manifest JSON schema, the version-comparison logic, the channel-selection logic, or the protocol version, and SHALL NOT add any server state or socket message.

#### Scenario: Stable channel checks the fork manifest

- **WHEN** a fork build on the stable channel runs a version check or `herdr update`
- **THEN** it fetches the fork's `website/latest.json` via raw GitHub
- **AND** it does not contact upstream `herdr.dev`

#### Scenario: Preview channel checks the fork manifest

- **WHEN** a fork build on the preview channel runs a version check or `herdr update`
- **THEN** it fetches the fork's `website/preview.json` via raw GitHub

#### Scenario: Remote provisioning uses the same fork manifests

- **WHEN** `herdr --remote` decides whether to install or replace a remote helper binary
- **THEN** it fetches the same fork manifest URL for the active channel as the local update path

### Requirement: Agent-detection catalog remains upstream

The agent-detection manifest catalog URL SHALL remain pointed at upstream `herdr.dev/agent-detection/index.toml`. Repointing the release manifests SHALL NOT change the agent-detection catalog source.

#### Scenario: Agent-detection updates are unaffected

- **WHEN** the agent-detection manifest updater fetches its catalog
- **THEN** it uses the upstream `herdr.dev/agent-detection` catalog
- **AND** the release-manifest repoint has no effect on it

### Requirement: The fork's stable manifest carries fork release data

The fork's `website/latest.json` SHALL be populated with fork release data — the fork version (`X.Y.Z-ac`), the fork's protocol version, and release asset URLs under `github.com/colangelo/herdr/releases/...` — rather than upstream release data. The fork's release flow SHALL update `website/latest.json` on each stable `-ac` release and commit it to the repository's default branch so the raw-GitHub manifest reflects the current fork release.

#### Scenario: A fork stable release publishes fork data

- **WHEN** a stable `-ac` release is cut
- **THEN** `website/latest.json` is rewritten with that release's fork version, fork protocol, and fork asset URLs
- **AND** the updated file is committed to the default branch so the raw-GitHub stable manifest serves it

#### Scenario: A fork user is offered a fork binary

- **WHEN** a fork stable build compares itself against the fork's `latest.json`
- **THEN** any offered update points at a `github.com/colangelo/herdr` release asset
- **AND** never at an upstream `ogulcancelik/herdr` asset

