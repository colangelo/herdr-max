# Probe: a core machine link, measured with a local `--via` (2026-11-22, m4m, hc 0.1.21)

The one probe the plan allowed. No `hc machine add` to a real Mac, no ssh, no Tailscale. The "machine" is a
second core on m4m (session `p5remote`, scratch XDG directories), reached through `relay.py` as the `--via`
command. The local core is `HC_SESSION=p5probe` with scratch `XDG_STATE_HOME`, `XDG_RUNTIME_DIR` and
`XDG_CONFIG_HOME` under `/tmp/p5h`. `hc schema` started that scratch core. Both cores were stopped with
`hc server stop` at the end. No default core and no `herdr` core was touched.

## Rerun

```sh
export P5_ROOT=/tmp/p5h HC_SESSION=p5probe XDG_STATE_HOME=/tmp/p5h/state XDG_RUNTIME_DIR=/tmp/p5h/run \
  XDG_CONFIG_HOME=/tmp/p5h/config
mkdir -p $P5_ROOT/{state,run,config,rstate,rrun}
hc machine add --json --name loop --session p5remote --via "python3 -I $PWD/relay.py"
hc run --json --bg --on loop --name sh1 sh -c 'while :; do sleep 3600; done'
# idle: read $P5_ROOT/relay.log
touch $P5_ROOT/dormant; pkill -f "$PWD/relay.py"      # go dormant: drop the link, refuse redials
trash $P5_ROOT/dormant; python3 -I rpc.py machine.connect '{"machine":"loop"}'   # wake
hc server stop; HC_SESSION=p5remote XDG_STATE_HOME=$P5_ROOT/rstate XDG_RUNTIME_DIR=$P5_ROOT/rrun hc server stop
```

Attach streams were read with the phase 0 client `~/.local/share/herdr-core-pilot/phase0/hcattach.py`
(`HC_SOCK=<scratch socket> hcattach.py loop/t1 --observe --verbose`).

## Results

| # | Question | Result |
|---|---|---|
| P1 | What runs on the far side | core gives the via command one shell line: `sh -c 'p=$(command -v herdr-core) \|\| p=$HOME/.local/bin/herdr-core; [ -x "$p" ] \|\| { echo "herdr-core is not installed here" >&2; exit 127; }; HC_SESSION=p5remote exec "$p" connect'`. `machine add` dials twice: a check connection that closes after 0.15 s, then the link. |
| P2 | Idle traffic on a connected link | **Every 3 s** the local core sends 70-71 bytes and the remote answers 283 bytes (no terminals) or 457 bytes (one terminal); every 15 s the remote sends a 67-byte heartbeat and the local core answers 13 bytes. Steady for 90 s with no viewer and no client. About 1 400 round trips an hour. |
| P3 | Can the link be paused through the API | No. The machine methods are `machine.add`, `machine.list`, `machine.connect`, `machine.rename`, `machine.rm` and `machine.exec`. `machine.connect`: "Dial a machine now instead of at its next retry". No idle or keepalive setting is documented. |
| P4 | What core does when the via command fails at once | It retries after 0.32, 0.56, 1.07, 2.06, 4.06, 8.07, 16.1, then every **30 s** (two 30.06 s gaps seen). Each retry only ran the local command. `machine.list` says `unreachable`, `why: "cannot reach <via>: the connection closed"`. |
| P5 | What a client sees while the machine is unreachable | `terminal.list` (and `hc ls`) drops the machine's terminals; `machines` lists it `unreachable`. The event stream sent **no event** for the loss, only heartbeats. |
| P6 | Wake | `machine.connect` returned `connected` at once; the terminal was back, its program untouched. The event stream got `missed` with a fresh snapshot. |
| P7 | An attach stream across a short drop (via allowed) | The stream stayed open: 41 bytes, then a 365-byte repaint 0.15 s after the kill. |
| P8 | An attach stream across a 25 s dormant drop | The stream stayed open. Core wrote `\x18\r\n[line to loop lost; reconnecting...]\r\n` into it at the drop, and on `machine.connect` a repaint that starts with CAN and a full mode reset (`\x18\x1b[!p...`). No CLOSE. |
| P9 | `hc machine login` without a terminal | refused: `"logging in needs a terminal"` (kind `usage`). `machine.connect` over the socket works without one. |

Not measured: how long an attach stream stays open past 25 s unreachable; what core does when the via command
hangs instead of failing; whether core ever classifies a via failure as `needs_login` (the 2, 5, 10 minute
ladder of `docs/machines.md`).

# Roundtable probes (2026-11-22, m4m, hc 0.1.21)

After the roundtable on ea5d5eb5 (findings F1-F12, `~/.local/state/herdr-core-p1/p5-roundtable.md`; its repros are in
`/tmp/arena-p5/candidate-1..4` and `/tmp/arena-p5/verify`). The scripts are in `fix/`, written for
`/tmp/arena-p5/fix` (`env.sh`: local core `p5fix-loc`, far core `p5fix-far`, scratch XDG directories, a clean
environment). `fix/corelink.py` is a prototype of `herdr core-link` (D2): it reads leases and the gate, takes
`dial.lock`, forks a watcher and then **execs ssh**, so the via pid core holds is ssh itself. Its ssh is
`fix/fakessh.py` (one process owning the link, modes `relay`, `hang`, `slow`, `banner`) or, with `h/ssh_real`, real
OpenSSH to `fix/sshd_fake.py` on 127.0.0.1:2298 (paramiko, a throwaway key, `ControlMaster=no`,
`ServerAliveInterval=5`, `ServerAliveCountMax=2`). The far core of R6 was started with five fake `HERDR_*`
variables, as a far core a bare `hc` in a herdr pane would carry. Both cores, the loopback server and every
terminal were stopped at the end; no default or `herdr` core was touched; nothing dialled another host.

| # | Finding | Question | Result |
|---|---|---|---|
| R1 | F1 | What tells a far terminal apart in local `terminal.list` | `terminal.list` takes only `all` and `labels`. A far record has the same keys as a local one and **no machine field**; only the address prefix differs (`t1` and `loop/t1` both listed with `herdr.session=herdr`, `herdr.pane_id=w1:p1`). The `events.subscribe` snapshot also lists `loop/t1`. |
| R2 | F2 | Can core-link classify a silent dial before core's ~19.8 s kill, and leave nothing | With the watcher's 15 s deadline: gate `auth_stalled` written at 15.0 s, ssh (the via pid) ended at 16.0 s, `machine.connect` answered `unreachable` after 16.3 s; the next dial was refused by the gate; no process left. `kill -9` of the via pid at 3 s (as core does): ssh is that pid, the watcher saw EOF and exited; nothing orphaned. |
| R3 | F3, F12 | How core runs the via per retry | Each retry runs the POSIX line (`sh -c '...'`), and when that fails, **a Windows line** 76-100 ms later (`cmd /d /c "if exist .\AppData\...herdr-core.exe ..."`). The prototype refuses the Windows line locally, so a retry dials once. A dormant machine: refused pairs at +8, +16, then every 30 s. |
| R4 | F3 | Concurrent `machine.connect` | Three at once: core ran **one** via, all three answered `connected` after 5.2 s. Three 1 s apart during a slow dial: core **ended the in-flight via and started a new one** for each call (each a fresh ssh, each its own auth); the lock handed over cleanly each time. |
| R5 | F10 | Can a dial avoid starting a stopped far core | Far guard `nc -U -w 2 <far socket> </dev/null` before core's line: 0 on a live socket, 1 on a missing one and on the **stale socket file a SIGKILLed core leaves**. With the far core stopped, two dials were refused (`herdr-core is not running there`, exit 69), the far socket stayed absent and no far core started. |
| R6 | F6 | Remote env | Unwrapped `terminal.run --on loop`: the program saw `HERDR_ENV HERDR_TAB_ID HERDR_SOCKET_PATH HERDR_WORKSPACE_ID HERDR_PANE_ID` (names only). Wrapped in `env -u HERDR_ENV -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH`: none. macOS `env -u NAME -u MISSING -- sh -c ...` exits 0. |
| R7 | F5 | Can the view type without taking the size (A = far herdr attached 100x30, B = view through the link at 160x50) | `observe: true`: no resize, but input (DATA and kind 6) **not delivered**. Plain attach: resized to 160x50 at attach. `resumed: true` + DATA: resized on the first key. **`resumed: true` + AUTOMATIC_INPUT (kind 6): input delivered, far size stayed 100x30 and followed A to 90x25.** A FOCUS frame from the `resumed` attach **resized the far terminal to 160x50**. Mouse reports as kind 6: delivered, no resize. |
| R8 | F5 | What the view's size follows | `size` events from `events.subscribe {terminal: "loop/tN"}` on the local core arrive within 2 ms of the far resize. The `resumed` view attached at the far size and echoing only a **changed** size as RESIZE: no extra size event, no storm (3 events for 3 far resizes, none in 3 s idle). When A detached, core applied the view's recorded size, which was then the last far size (120x40 kept); without the echo it was the view's stale attach size (160x50). |
| R9 | F9, F11 | Real OpenSSH with `ControlMaster=no` and keepalives, then the loopback server SIGSTOPped | No control socket anywhere. Frozen at t=0: ssh exited at 15 s (keepalive), the redial ended at 25 s (`ConnectTimeout=10` covers the banner), machine `unreachable` at 26 s. `terminal.list` on the primary took **3.00 s per call** while `connected` and `connecting`, 0.00 s once `unreachable`. Healthy: the marker came 39 ms after start. |
| R10 | F7 | `machine.exec` on a dormant machine; the CLI flow | Dormant: `unavailable` at once. CLI flow (write `cli-<pid>` lease, `machine.connect`, `machine.exec`, remove the lease): connected in 0.16 s, exec answered `code 0`, `truncated: false`, `timed_out: false`; the link ended within 7 s of the lease's removal. |

Not measured: the 1Password agent's prompt per dial with `ControlMaster=no` (open question 3); `nc -U` on the
Linux VMs' images (D2 falls back to a socket-file test there).

# M1: which sandboxes reach a core socket (2026-11-22, m4m, hc 0.1.21)

Security review M1 (security `docs/reviews/2026-11-22-herdr-core-link.md`): while a lease holds, any process that
can open the primary core socket can run commands on the far machine through the link. Measured against two
throwaway cores (`HC_SESSION=m1sandbox`, socket under `/tmp`; `m1sandbox2`, socket in the home class
`~/.local/state/.../herdr-core/<s>/<s>.sock`, where a real primary's lives on macOS when `XDG_RUNTIME_DIR` is
unset), with a probe that connects and sends `server.status`. Where a sandbox refused, the same socket was opened
under it with an explicit grant first, and connected, so each refusal is the sandbox's. Both cores were stopped.

| Sandbox | Started as | `/tmp` socket | home-class socket | Notes |
|---|---|---|---|---|
| Claude Code Bash tool | probe run directly | connects | connects | not sandboxed here: no `sandbox` setting, writes to `~/Library` work |
| nono `claude-gpt` (GPT seats) | `nono run -p <copy without credentials> --allow-cwd` | refused | refused | nono's network proxy blocks it; `--allow-unix-socket` connects |
| nono `pi-lab` (Pi, local models) | as above, credentials and approvals removed | connects | connects | with a credential the proxy turns on and both are refused; the real profile has one, so refused (inferred) |
| nono built-in `default` | `nono run -p default` | connects | connects | |
| `security/harness/review-guard/review.sb` | `sandbox-exec -f ...` | refused | refused | unix sockets only under its TMPD, NOTES, WORK |
| `direction/e2e/browser-use/sandbox.sb` | `sandbox-exec -f ...` | refused | refused | unix sockets only under its TMP |
| Codex 0.170.0 seatbelt (default, workspace-write, read-only) | `codex sandbox ...` | refused | refused | `--allow-unix-socket <dir>` opens that folder only |
| Codex workspace-write with network | `-c sandbox_workspace_write.network_access=true` | connects | connects | any Codex run with network on reaches the core |

On macOS the network rules, not the file grants, decide `connect()` on a unix socket; no sandbox treated the two
path classes differently. So the link reaches the far machine from: Claude Code sessions (unsandboxed), nono's
`default` profile and any nono profile without its proxy, and Codex with network on.

Not measured: Claude Code's own sandbox mode (off here, `srt` not installed, turning it on needs a `claude`
launch); the real `claude-gpt` and `pi-lab` profiles with their credentials (that would fetch live keys); Codex
named permission profiles (`lab-fixture`); the bmlab local-model harness, which lives on ac-mbm5 (needs ssh).

# PR D: what the view reads of a machine (2026-11-23, m4m, hc 0.1.21)

Two throwaway cores (`d300loc`, `d300far`, scratch XDG folders), the far one added to the near one with `hc machine
add --via` and a relay script; both stopped after. Nothing dialled another host.

| # | Question | Result |
|---|---|---|
| D1 | Does a far `doc.set` reach the near core's events | Yes, as `{"method":"event","params":{"type":"doc","machine":"loop","data":{"name":"herdr-app","version":2},"seq":3}}`, 68 ms after the far write, but **only on a subscription with no filter**: one with a label filter, or with `terminal`, got none. This Mac's own `doc.set` gives the same event without `machine`. |
| D2 | `doc.get {name, on}` | the far document and its version, through the link. |
| D3 | A far resize on an unfiltered subscription | `{"type":"size","terminal":"loop/t4","data":{"cols":90,"rows":25}}`: the address is qualified, as in the snapshot (`loop/t1` ...). |

So the view server subscribes once, with no filter, and keeps its own machine's `doc`, `size`, `status` and
`finished`/`removed` facts (`core_backend/view_link.rs`).
