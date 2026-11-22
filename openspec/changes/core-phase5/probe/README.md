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
