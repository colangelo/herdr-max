# remote-binary-discovery

## ADDED Requirements

### Requirement: Discovery follows the invoked binary name
When preparing a remote host, the client SHALL search for a remote herdr binary under the name it was itself invoked as, and SHALL also search under the canonical name `herdr`, most specific first. The search SHALL apply both names to the remote `PATH` lookup and to every known install path probed on the remote.

#### Scenario: Beta client finds the remote beta binary
- **WHEN** a client invoked as `herdr-beta` prepares a remote host that has `herdr-beta` installed at the same version and protocol
- **THEN** discovery matches that binary and no install is performed

#### Scenario: Canonical name still probed
- **WHEN** a client invoked under any name prepares a remote host that only has a matching `herdr` binary
- **THEN** discovery matches that binary, so single-channel setups keep working

#### Scenario: A different channel is not mistaken for a match
- **WHEN** the remote has a `herdr` binary of a different version than the invoked client
- **THEN** it is not treated as a match, and the normal install path is taken

### Requirement: Managed install path follows the binary name
When no matching remote binary is found and herdr installs its own copy, the install target SHALL be `~/.local/bin/<invoked name>`, so binaries of different channels do not overwrite one another on the remote.

#### Scenario: Beta installs under its own name
- **WHEN** a client invoked as `herdr-beta` must install onto a remote
- **THEN** the binary is installed at `~/.local/bin/herdr-beta`, leaving any `~/.local/bin/herdr` untouched

### Requirement: Binary names are shell-safe
Binary names are interpolated into commands executed on the remote host, so only names consisting of ASCII alphanumerics, `.`, `_`, and `-`, not starting with `-`, SHALL be used. Any other name SHALL fall back to the canonical `herdr`.

#### Scenario: Metacharacters are rejected
- **WHEN** the invoked binary name contains shell metacharacters such as `;`, `$`, or `"`
- **THEN** that name is not used for discovery and the canonical name is used instead

### Requirement: Matching remote server is left running
When discovery finds a remote binary matching the client's version and protocol, the client SHALL attach without installing and without stopping or restarting the running remote server.

#### Scenario: Same build on both machines attaches non-destructively
- **WHEN** both machines run the same build and a remote attach is initiated
- **THEN** no install prompt is shown, the remote server is not stopped, and remote pane processes keep running
