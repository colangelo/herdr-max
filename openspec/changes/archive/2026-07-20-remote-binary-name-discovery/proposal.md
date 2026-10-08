# Remote Binary Name Discovery

> Backfilled artifact: implemented and verified in `6baf691a` before these
> artifacts were written. Recorded so the capability lands in `openspec/specs/`.

## Why

`herdr-beta --remote <target>` reinstalled and offered to **stop the running remote server** even when both machines were on the identical build — destroying remote panes for nothing. Remote binary discovery hardcoded the name `herdr` (`command -v herdr` plus `herdr`-named install paths), so on a machine with both channels installed it found the stable `herdr`, whose version can never match a `herdr-beta` client, and fell through to install. The version shown in the prompt came from the *running server* (read over the socket, name-agnostic), which is why it looked identical and the prompt looked nonsensical.

Decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/22.

## What Changes

- Remote binary discovery probes the **name the client was invoked as** (e.g. `herdr-beta`) first, then canonical `herdr` as a fallback, across `command -v` and every known install path.
- The managed install target follows that name (`~/.local/bin/<name>`), so channels stay separate on the remote instead of one overwriting the other.
- Binary names are sanitized to plain `[A-Za-z0-9._-]` before being interpolated into remote shell commands.

## Capabilities

### New Capabilities

- `remote-binary-discovery`: How `--remote` locates, matches, and installs the herdr binary on a remote host.

### Modified Capabilities

<!-- none -->

## Impact

- `src/remote/unix.rs`: `RemoteHerdr::for_platform_named`, `remote_binary_names`, `sanitized_binary_name`, name-parameterised `remote_binary_on_path_any` and `known_remote_binary_candidate_script`, and the install-name resolution in `prepare_remote_herdr`.
- Behavior for a stable `herdr` client is unchanged.
