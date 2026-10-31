# Sandbox pilot for the herdr sessions

One week of Claude Code's Bash sandbox for the sessions in this checkout (herdr-helper, herdr-wrk). ac said yes on
2026-10-31 (asks row gestore-lab.a1030-20, signed). The plan is planner `docs/ideas/2026-10-30-sandboxing-agents-on-macs.md`
§ The plan. Tracked in https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/256.

The profile is the `sandbox` block of `../settings.json`. `strictAllowlist` is not set there, because a repository file
cannot set it. manager-lab-hlp builds each session's launch file from the committed `allowedDomains`.

Blocked calls are logged by `log-violation.sh` (PostToolUse and PostToolUseFailure, so blocks inside a command that
exits 0 are kept too) to `~/.local/state/herdr-sandbox/violations.jsonl`. The logger is copied from security's
`checks/sandbox-pilot/log-violation.sh`.

Rollback: remove the `sandbox` block and the two hooks, and restart with plain settings.

## Why each entry is there

**Pseudo-terminals and sockets** (herdr is a terminal multiplexer; its own tests are the hard case):

- `allowPty`: every pane is a PTY, and 88 of 134 cli tests failed with `failed to openpty` without it.
- `network.allowAllUnixSockets`: the tests bind and connect server sockets in fresh random `/tmp/hcli-*` folders,
  which a fixed path list cannot name (39 tests failed on `UnixListener::bind`).
- `network.allowUnixSockets`: only `/private/tmp/herdr-tests`, the folder every integration test's sockets live in.
  Not listed, pending security: the live server's `~/.config/herdr/herdr.sock` (it drives every pane, and can type
  into an unsandboxed shell) and the 1Password SSH agent socket (it signs with the ssh keys). Nothing here signs
  commits or uses ssh. Without the server socket, `herdr-beta` calls from a sandboxed command fail; they are asked
  for through manager-lab.
- `network.allowLocalBinding`: the live-handoff and client tests listen on localhost TCP ports.

**Writes** (the checkout itself is writable by default):

- `~/dev/worktrees/herdr`: task worktrees made with `wt`.
- `~/.cargo`, `~/.rustup`: the cargo registry and git cache, installed tools, and the pinned toolchain.
- `~/.cache/zig`: zig's global cache, used to build the vendored libghostty-vt.
- `~/.cache/uv`: uv's cache for the CONTEXT scripts (asks.py and reconcile.py are excluded, other uv scripts are not).
- `/tmp`, `/private/tmp`: the integration tests' `hcli-*` folders and the shared `/tmp/m4m-heavy.lock`.
- `~/.config/herdr-dev`: state of a debug server started with `cargo run`.
- `~/.local/state/agent-handover/herdr-helper`: the session's keep and handover notes.
- `~/.local/state/herdr-beta-local`: `just beta-local` builds and signs the beta there.
- `~/.local/state/herdr-sync`: the upstream sync runner's state.
- `~/.local/state/model-routing`, `~/.local/state/agent-bell`: the route and bell tools write their state there.
- `~/_sync/dev/.agent-state/reconcile`: reconcile records.

Not granted: `~/.Trash`. The sandbox refuses it even when listed (clipper measured it), so `trash` does not work from a
sandboxed command; scratch files go in `$TMPDIR`. The whole of `/tmp` is on the end-of-week review list: the tests'
`hcli-*` folders and the heavy-cargo lock are the reason.

**Hosts** (`network.allowedDomains`):

- `github.com`, `api.github.com`, `uploads.github.com`, `codeload.github.com`, `objects.githubusercontent.com`,
  `release-assets.githubusercontent.com`, `raw.githubusercontent.com`: push and fetch `origin`, `gh` for the beta
  release, its assets and the Homebrew tap formula.
- `gitea.cat-bluegill.ts.net`: the `internal` remote and the fork's issue tracker API.
- `asks.cat-bluegill.ts.net`: the asks page.
- `index.crates.io`, `static.crates.io`, `crates.io`: cargo dependency downloads.
- `deps.files.ghostty.org`: libghostty-vt's zig dependencies on a cache miss.
- `pypi.org`, `files.pythonhosted.org`: dependencies of uv scripts.

**Excluded commands** (run outside the sandbox): `agent-bell`, `nats`, and infra's relay tools, plus asks.py and
reconcile.py. They speak the NATS protocol to `bus.cat-bluegill.ts.net:4222`, which the HTTP/SOCKS proxy does not
carry. A command keeps the sandbox when it contains a pipe, a `cd` or `$(…)`, so these run as plain commands.

## Findings so far

- `nice` fails (`setpriority: Operation not permitted`) and the command then runs at normal priority. That breaks the
  house rule that heavy cargo runs niced on m4m. Raised with manager-lab.
- `allowPty` set in this file did not take effect in the running session (unix sockets did). Checked again after the
  restart with the launch file.
- `allowUnixSockets` takes a folder and allows every socket below it, nested folders included; a wildcard entry
  (`/private/tmp/hsbx-glob-*`) does not work. So every integration test's temp dir lives under `/tmp/herdr-tests`
  (`tests/support::test_tmp_dir`), and only that folder is listed. manager-lab refused `allowAllUnixSockets`: it also
  opens docker.sock, the agent sockets and herdr's own server.
- Measured in the sandbox before the restart (2026-10-31): cli tests 46/134 with no socket entry, 86/134 with the
  folder (the 48 left all need PTYs). Unit tests 4598/5019, and 4844/5019 with `TMPDIR=/tmp/herdr-tests`. About 80
  unit tests still bind sockets at hard-coded `/tmp` paths in `src/` (for example `/tmp/hu-<name>.sock` in
  `update.rs`). How to run `just check` in the pilot is manager-lab's call.
- An excluded command keeps the sandbox when its line holds a pipe, a `cd` or `$(…)`: `agent-bell … | tail -1` failed
  with `bus unreachable`. Bells go out as a plain command.
