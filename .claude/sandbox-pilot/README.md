# Sandbox pilot for the herdr sessions

A one-week trial of Claude Code's Bash sandbox for the sessions in this checkout (herdr-helper, herdr-wrk). ac said yes on
2026-10-31 (asks row gestore-lab.a1030-20, signed). The plan is planner `docs/ideas/2026-10-30-sandboxing-agents-on-macs.md`
§ The plan. Tracked in https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/256.

**Result: herdr left the pilot on 2026-10-31 at 11:16Z (manager-lab's choice B).** Inside the sandbox, nobody in the repo
can create a pseudo-terminal (PTY), so nobody can run `just check` there (see Findings). The `sandbox` block and its two
hooks are taken out of `../settings.json`. The sessions go back to plain `aiproxy.settings.json`. security records the
same result as PF10 in ac/security `checks/sandbox-pilot/README.md`. herdr re-enters only on manager-lab's word, through
security, once a documented `allowPty` route exists. This file and `log-violation.sh` stay as the record.

The profile lived in the `sandbox` block of `../settings.json` (last in commit b31e5528). `strictAllowlist` was not set
there, because a repository file cannot set it. manager-lab-hlp built each session's launch file from the committed
`allowedDomains`.

Blocked calls were logged by `log-violation.sh` (PostToolUse and PostToolUseFailure, so blocks inside a command that
exits 0 were kept too) to `~/.local/state/herdr-sandbox/violations.jsonl`. The logger is copied from security's
`checks/sandbox-pilot/log-violation.sh`.

## Why each entry was there

**Pseudo-terminals and sockets** (herdr is a terminal multiplexer, so its own tests are the hard case):

- `allowPty`: every pane is a PTY, and 88 of 134 cli tests failed with `failed to openpty` without it. It did not help
  (Findings).
- `network.allowUnixSockets`: only `/private/tmp/herdr-tests`, the folder every integration test's sockets live in
  (PF9, manager-lab and security). Left out on purpose: the live server's `~/.config/herdr/herdr.sock` (it drives every
  pane and can type into an unsandboxed shell; security: a socket allow is never read-only), and the 1Password SSH agent
  socket (it signs with the ssh keys). `allowAllUnixSockets` was refused (PF7): it also opens docker.sock, the agent
  sockets and herdr's own server.
- `network.allowLocalBinding`: the live-handoff and client tests listen on localhost TCP ports.

**Writes** (the checkout itself is writable by default):

- `~/dev/worktrees/herdr`: task worktrees made with `wt`.
- `~/.cargo`, `~/.rustup`: the cargo registry and git cache, installed tools, and the pinned toolchain.
- `~/.cache/zig`: zig's global cache, used to build the vendored libghostty-vt.
- `~/.cache/uv`: uv's cache for the CONTEXT scripts (asks.py and reconcile.py are excluded, other uv scripts are not).
- `/tmp`, `/private/tmp`: the integration tests' folders and the shared `/tmp/m4m-heavy.lock`.
- `~/.config/herdr-dev`: state of a debug server started with `cargo run`.
- `~/.local/state/agent-handover/herdr-helper`: the session's keep and handover notes.
- `~/.local/state/herdr-beta-local`: `just beta-local` builds and signs the beta there.
- `~/.local/state/herdr-sync`: the upstream sync runner's state.
- `~/.local/state/model-routing`, `~/.local/state/agent-bell`: the route and bell tools write their state there.
- `~/_sync/dev/.agent-state/reconcile`: reconcile records.

Not granted: `~/.Trash`. The sandbox refuses it even when listed (clipper measured it), so `trash` does not work from a
sandboxed command. Scratch files go in `$TMPDIR`.

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
reconcile.py. They speak the NATS protocol to `bus.cat-bluegill.ts.net:4222`, which the HTTP/SOCKS proxy does not carry.

## Findings

- **PTYs are refused, and `allowPty` does nothing we can see.** `posix_openpt` works, but `grantpt` fails with
  `Operation not permitted`. That held with `sandbox.allowPty: true` in this file and in the launch file, after a fresh
  start, and with `/dev/ptmx` and `/dev/ttys*` added to `allowWrite` (taken out again). The key appears only in the
  sandbox engine's own schema.
- **`allowPty` is not documented (checked 2026-10-31).** The settings reference's index lists 39 `sandbox.*` keys and
  none is `allowPty`. The sandboxing page never mentions a PTY, `openpty` or `grantpt`. The Claude Code changelog
  mentions only `winpty` (2.1.316, a Git Bash fix on Windows). So there is no documented route to PTYs in the sandbox.
- **Controls that passed (11:16Z):** a write outside the allowed folders was blocked; an off-list host (example.com) got
  403 while GitHub answered; a plain bell to manager-lab arrived; `git push` to gitea worked (throwaway branch, deleted).
- **Sockets:** `allowUnixSockets` takes a folder and allows every socket below it, nested folders included. A wildcard
  entry (`/private/tmp/hsbx-glob-*`) does not work. So every integration test's temp dir lives under `/tmp/herdr-tests`
  (`tests/support::test_tmp_dir`). Measured in the sandbox: cli tests 46/134 with no socket entry, 86/134 with the
  folder (the 48 left all need PTYs). Unit tests 4598/5019, and 4844/5019 with `TMPDIR=/tmp/herdr-tests`. About 80 unit
  tests still bind sockets at hard-coded `/tmp` paths in `src/` (for example `/tmp/hu-<name>.sock` in `update.rs`):
  https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/257, worth doing anyway for parallel-run collisions.
- **`nice` fails** (`setpriority: Operation not permitted`) and the command then runs at normal priority. That breaks the
  house rule that heavy cargo runs niced on m4m. manager-lab accepted un-niced heavy cargo under the lock for the pilot.
- **git's credential-store write after a push is blocked** (the `~/.git-credentials` lock). The push itself succeeds;
  the log gets noise each time (security PF4).
- **Excluded commands keep the sandbox when the line holds a pipe, a `cd`, a `;` chain, `$(…)` or a redirect.**
  `agent-bell … | tail -1` failed with `bus unreachable`. Bells and asks.py calls go out as one plain command.
- **`herdr-beta` cannot reach the live server from the sandbox** (herdr.sock is not listed). Moot after the rollback.
