#!/usr/bin/env bash
# The view server (openspec/changes/core-phase5, PR D, D6). Two scratch cores
# on this Mac stand in for two Macs: the far one is reached by `hc machine add
# --via` with a local relay script, never ssh. A far herdr server (debug,
# scratch session) publishes its spaces to the far core; the view server of
# the near session shows them through the near core's machine link, and a TUI
# client in a private tmux server drives it. A user's herdr and cores are
# never reached; every core call carries a scratch session.
#
# V15 (the far layout reaches the view within 2 s), V14 (typing, a paste, a
# click and focus in the view never resize the far terminal; only
# AUTOMATIC_INPUT, CREDIT and one RESIZE per changed far size go from the
# view, read by a frame-logging proxy between the view and the near core),
# V30 (an idle view sends nothing but CREDIT for 60 s; when the far herdr
# drops, core keeps the far size), V17 (closing a pane in the view and
# stopping the view leave the far program and pane as they were), D4's
# `--remote core:<machine>`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERDR_BIN="${HERDR_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/debug/herdr}"
IDLE_SECS="${V30_IDLE_SECS:-60}"
BASE="$(mktemp -d /tmp/hcv.XXXXXX)"
case "$BASE" in
  /tmp/hcv.*) ;;
  *) echo "refusing scratch folder $BASE" >&2; exit 1 ;;
esac
TMUX_SOCK="hcv$$"
NEAR_CORE="herdrdev-vw"
FAR_CORE="herdrdev-far"
NEAR_SOCK="$BASE/r/herdr-core/$NEAR_CORE.sock"
FAR_SOCK="$BASE/fr/herdr-core/$FAR_CORE.sock"
PROXY_SOCK="$BASE/px/core.sock"
FRAMES="$BASE/frames.log"
FAR_API="$BASE/fc/herdr-dev/sessions/far/herdr.sock"
VIEW_SOCK="$BASE/c/herdr-dev/sessions/vw/herdr-view-loop.sock"
VIEW_LOG="$BASE/c/herdr-dev/sessions/vw/herdr-view-loop.log"
FAR_LOG_DIR="$BASE/fc/herdr-dev/sessions/far"
VIEW_PID=""
PROXY_PID=""
FAR_PID=""

clean_env() {
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
    -u CLAUDECODE -u TMUX SHELL=/bin/sh "$@"
}
near() { clean_env XDG_CONFIG_HOME="$BASE/c" XDG_RUNTIME_DIR="$BASE/r" XDG_STATE_HOME="$BASE/s" "$@"; }
far() { clean_env XDG_CONFIG_HOME="$BASE/fc" XDG_RUNTIME_DIR="$BASE/fr" XDG_STATE_HOME="$BASE/fs" "$@"; }
hc_near() { near HC_SESSION="$NEAR_CORE" hc "$@"; }
hc_far() { far HC_SESSION="$FAR_CORE" hc "$@"; }
tm() { tmux -L "$TMUX_SOCK" -f /dev/null "$@"; }

cleanup() {
  set +e
  tm kill-server >/dev/null 2>&1
  [[ -n "$VIEW_PID" ]] && kill "$VIEW_PID" 2>/dev/null
  far "$HERDR_BIN" --session far server stop >/dev/null 2>&1
  [[ -n "$FAR_PID" ]] && kill "$FAR_PID" 2>/dev/null
  [[ -n "$PROXY_PID" ]] && kill "$PROXY_PID" 2>/dev/null
  # Only a running one: any hc call would start the session.
  [[ -S "$NEAR_SOCK" ]] && hc_near server stop >/dev/null 2>&1
  [[ -S "$FAR_SOCK" ]] && hc_far server stop >/dev/null 2>&1
  for pid in $(jobs -p); do kill "$pid" 2>/dev/null; done
  if [[ -n "${KEEP_SCRATCH:-}" ]]; then echo "kept $BASE" >&2; else rm -rf "$BASE"; fi
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  echo "--- view log tail ---" >&2
  tail -n 40 "$VIEW_LOG" 2>/dev/null >&2 || true
  echo "--- view screen ---" >&2
  tm capture-pane -p -t view 2>/dev/null >&2 || true
  cat "$BASE/client.err" 2>/dev/null >&2 || true
  exit 1
}

now() { python3 -I -c 'import time; print(f"{time.time():.3f}")'; }
wait_until() {
  local what="$1" tries="${WAIT_TRIES:-200}"
  shift
  for _ in $(seq 1 "$tries"); do
    if "$@"; then return 0; fi
    sleep 0.05
  done
  fail "timed out waiting for: $what"
}

mkdir -p "$BASE"/{c/herdr-dev,r,s,fc/herdr-dev,fr,fs,bin}
mkdir -m 700 "$BASE/px"
printf 'onboarding = false\n\n[experimental]\nruntime = "core"\n' > "$BASE/c/herdr-dev/config.toml"
printf 'onboarding = false\n\n[server]\nheadless_cols = 104\nheadless_rows = 34\n\n[experimental]\nruntime = "core"\ncore_spaces_doc = true\n' \
  > "$BASE/fc/herdr-dev/config.toml"

# The relay: core hands it one shell line, which runs against the far core.
cat > "$BASE/bin/via.sh" <<VIA
#!/bin/sh
for line; do :; done
exec env -u HC_SESSION -u HC_SOCKET XDG_RUNTIME_DIR="$BASE/fr" XDG_STATE_HOME="$BASE/fs" sh -c "\$line"
VIA
chmod +x "$BASE/bin/via.sh"

# One JSON-RPC call on a core socket (plain mode).
cat > "$BASE/bin/rpc.py" <<'PY'
import json, socket, sys
path, method, params = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
s = socket.socket(socket.AF_UNIX); s.connect(path); f = s.makefile("rb")
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {}}) + "\n").encode())
f.readline()
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n").encode())
reply = json.loads(f.readline())
if "error" in reply:
    raise SystemExit(f"rpc error: {reply['error']}")
print(json.dumps(reply["result"]))
PY

# A frame-logging relay between the view and the near core: every frame the
# view sends, after its hello line, as "<unix time> <channel> <kind> <len>
# <first 8 payload bytes as hex>".
cat > "$BASE/bin/proxy.py" <<'PY'
import os, socket, struct, sys, threading, time
listen, target, log = sys.argv[1], sys.argv[2], sys.argv[3]
server = socket.socket(socket.AF_UNIX); server.bind(listen); os.chmod(listen, 0o600); server.listen(16)
out = open(log, "a", buffering=1)
lock = threading.Lock()
def pump(src, dst):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    for sock in (src, dst):
        try: sock.shutdown(socket.SHUT_RDWR)
        except OSError: pass
def client_side(client, core):
    buf = b""
    hello = True
    try:
        while True:
            data = client.recv(65536)
            if not data:
                break
            core.sendall(data)
            buf += data
            if hello:
                if b"\n" not in buf:
                    continue
                buf = buf.split(b"\n", 1)[1]
                hello = False
            while len(buf) >= 9:
                channel, kind, size = struct.unpack("<IBI", buf[:9])
                if len(buf) < 9 + size:
                    break
                payload = buf[9:9 + size]
                with lock:
                    out.write(f"{time.time():.3f} {channel} {kind} {size} {payload[:8].hex()}\n")
                buf = buf[9 + size:]
    except OSError:
        pass
    for sock in (client, core):
        try: sock.shutdown(socket.SHUT_RDWR)
        except OSError: pass
while True:
    client, _ = server.accept()
    core = socket.socket(socket.AF_UNIX); core.connect(target)
    threading.Thread(target=client_side, args=(client, core), daemon=True).start()
    threading.Thread(target=pump, args=(core, client), daemon=True).start()
PY

# A second viewer of the far terminal, directly on the far core, standing in
# for the far herdr's own client resizing: a plain attach at a size takes it.
cat > "$BASE/bin/farviewer.py" <<'PY'
import json, socket, struct, sys, time
path, address, cols, rows, hold = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
s = socket.socket(socket.AF_UNIX); s.connect(path); f = s.makefile("rb")
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"framed": True}}) + "\n").encode())
f.readline()
req = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "terminal.attach",
                  "params": {"terminal": address, "cols": cols, "rows": rows}}).encode()
s.sendall(struct.pack("<IBI", 0, 0, len(req)) + req)
s.settimeout(0.2)
import os
while os.path.exists(hold):
    try:
        data = s.recv(65536)
        if not data:
            break
    except socket.timeout:
        pass
PY

api_far() {
  python3 -I - "$FAR_API" "$1" <<'PY'
import json, socket, sys
path, body = sys.argv[1], sys.argv[2]
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(path)
client.sendall(body.encode() + b"\n")
response = b""
while not response.endswith(b"\n"):
    chunk = client.recv(65536)
    if not chunk:
        break
    response += chunk
reply = json.loads(response)
if "error" in reply:
    raise SystemExit(f"api error: {reply['error']}")
print(json.dumps(reply["result"]))
PY
}
far_up() { [[ -S "$FAR_API" ]] && api_far '{"id":"v","method":"ping","params":{}}' >/dev/null 2>&1; }
far_size() {
  python3 -I "$BASE/bin/rpc.py" "$FAR_SOCK" terminal.get "{\"terminal\":\"$1\"}" \
    | python3 -I -c 'import json,sys; r=json.load(sys.stdin); print(str(r["cols"]) + "x" + str(r["rows"]))'
}
far_pid() {
  python3 -I "$BASE/bin/rpc.py" "$FAR_SOCK" terminal.get "{\"terminal\":\"$1\"}" \
    | python3 -I -c 'import json,sys; r=json.load(sys.stdin); print(r["pid"] if r["state"]=="live" else "")'
}
layout_lines() { grep -c '"view layout"\|view layout' "$VIEW_LOG" 2>/dev/null || true; }
last_layout() { grep 'view layout' "$VIEW_LOG" | tail -n 1; }
frames_from() { tail -n +"$(( $1 + 1 ))" "$FRAMES"; }
frame_count() { wc -l < "$FRAMES" | tr -d ' '; }

echo "herdr: $HERDR_BIN"
echo "scratch: $BASE"

# The far Mac: its core and its herdr, which publishes its spaces.
far "$HERDR_BIN" --session far server >/dev/null 2>&1 &
FAR_PID=$!
wait_until "far herdr" far_up
WS="$(api_far "{\"id\":\"v\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true,\"label\":\"far work\"}}")"
WS_ID="$(python3 -I -c 'import json,sys; print(json.loads(sys.argv[1])["workspace"]["workspace_id"])' "$WS")"
PANE="$(python3 -I -c 'import json,sys; print(json.loads(sys.argv[1])["root_pane"]["pane_id"])' "$WS")"
FAR_ADDR="$(api_far "{\"id\":\"v\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"$PANE\"}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["pane"]["core_address"])')"
[[ "$FAR_ADDR" =~ ^t[0-9]+$ ]] || fail "the far pane is not a core terminal: $FAR_ADDR"
# Every byte the far program reads goes to a file: raw, so each key counts.
api_far "$(python3 -I -c 'import json,sys; print(json.dumps({"id":"v","method":"pane.send_input","params":{"pane_id":sys.argv[1],"text":"stty raw -echo; exec cat > "+sys.argv[2],"keys":["Enter"]}}))' "$PANE" "$BASE/far-input.log")" >/dev/null
far_cat() { [[ -f "$BASE/far-input.log" ]]; }
wait_until "the far program" far_cat
FAR_PROGRAM="$(far_pid "$FAR_ADDR")"
[[ -n "$FAR_PROGRAM" ]] || fail "the far terminal is not live"
echo "ok: far herdr pane $PANE is far terminal $FAR_ADDR (pid $FAR_PROGRAM)"

# The near Mac: its core has the far one as machine `loop`.
hc_near machine add --json --name loop --session "$FAR_CORE" --via "$BASE/bin/via.sh" >/dev/null \
  || fail "machine add failed"
python3 -I "$BASE/bin/proxy.py" "$PROXY_SOCK" "$NEAR_SOCK" "$FRAMES" &
PROXY_PID=$!
touch "$FRAMES"
proxy_up() { [[ -S "$PROXY_SOCK" ]]; }
wait_until "frame proxy" proxy_up

# The view server, through the proxy.
near HC_SOCKET="$PROXY_SOCK" "$HERDR_BIN" --session vw server --view loop >/dev/null 2>&1 &
VIEW_PID=$!
view_up() { [[ -S "$VIEW_SOCK" ]]; }
wait_until "view socket" view_up
has_layout() { [[ "$(layout_lines)" -ge 1 ]]; }
wait_until "the far layout in the view" has_layout
last_layout | grep -q "spaces=1" || fail "expected one space: $(last_layout)"

# A person's client on it: `--remote core:loop` (D4) in a sized tmux.
tm new-session -d -s view -x 170 -y 55 \
  "$(printf '%q ' env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
     -u TMUX -u HC_SOCKET -u HC_SESSION XDG_CONFIG_HOME="$BASE/c" XDG_RUNTIME_DIR="$BASE/r" \
     XDG_STATE_HOME="$BASE/s" TERM=xterm-256color "$HERDR_BIN" --session vw --remote core:loop) 2>$BASE/client.err"
tm set-option -t view remain-on-exit on >/dev/null
shows_far() { tm capture-pane -p -t view 2>/dev/null | grep -q "far work"; }
WAIT_TRIES=400 wait_until "the view client showing the far space" shows_far
echo "ok: the view shows the far space through --remote core:loop"

# --- V15: the far layout follows within 2 s of its change ----------------
before="$(layout_lines)"
t0="$(now)"
api_far "{\"id\":\"v\",\"method\":\"tab.create\",\"params\":{\"workspace_id\":\"$WS_ID\",\"label\":\"logs\",\"focus\":false}}" >/dev/null
more_layouts() { [[ "$(layout_lines)" -gt "$before" ]] && last_layout | grep -q "tabs=2"; }
wait_until "the new tab in the view" more_layouts
t1="$(now)"
before="$(layout_lines)"
api_far "{\"id\":\"v\",\"method\":\"pane.split\",\"params\":{\"target_pane_id\":\"$PANE\",\"direction\":\"right\",\"focus\":false}}" >/dev/null
three_panes() { [[ "$(layout_lines)" -gt "$before" ]] && last_layout | grep -q "panes=3"; }
wait_until "the new split in the view" three_panes
t2="$(now)"
python3 -I - "$t0" "$t1" "$t2" <<'PY' || fail "V15: a change took over 2 s to reach the view"
import sys
t0, t1, t2 = map(float, sys.argv[1:])
print(f"V15: new tab in the view after {t1 - t0:.2f} s, new split after {t2 - t1:.2f} s")
raise SystemExit(0 if t1 - t0 <= 2.0 and t2 - t1 <= 2.0 else 1)
PY
shows_logs() { tm capture-pane -p -t view | grep -q "logs"; }
wait_until "the new tab's name on screen" shows_logs
echo "ok: V15 the view follows the far tab and split"

# --- V14: the view never resizes the far side ----------------------------
FAR_SIZE0="$(far_size "$FAR_ADDR")"
sleep 1
mark="$(frame_count)"
tm send-keys -t view -l "$(printf 'a%.0s' $(seq 1 50))"
tm set-buffer -b vb "pasted-text"
tm paste-buffer -p -b vb -t view
tm send-keys -t view -l $'\e[<0;40;12M\e[<0;40;12m'
tm send-keys -t view -l $'\e[O'
tm send-keys -t view -l $'\e[I'
fifty() { [[ "$(tr -cd 'a' < "$BASE/far-input.log" | wc -c | tr -d ' ')" -ge 50 ]]; }
wait_until "50 keys at the far program" fifty
pasted() { grep -q "pasted-text" "$BASE/far-input.log"; }
wait_until "the paste at the far program" pasted
sleep 1
[[ "$(far_size "$FAR_ADDR")" == "$FAR_SIZE0" ]] || fail "V14: typing in the view resized the far terminal from $FAR_SIZE0 to $(far_size "$FAR_ADDR")"
echo "ok: V14 50 keys, a paste, a click and focus reached the far program; the far terminal kept $FAR_SIZE0"

# The far side resizes (another viewer there takes 90x25).
touch "$BASE/hold"
python3 -I "$BASE/bin/farviewer.py" "$FAR_SOCK" "$FAR_ADDR" 90 25 "$BASE/hold" &
VIEWER_PID=$!
resized() { [[ "$(far_size "$FAR_ADDR")" == "90x25" ]]; }
wait_until "the far terminal at 90x25" resized
sleep 1
tm send-keys -t view -l "bbbbbbbbbb"
ten_b() { [[ "$(tr -cd 'b' < "$BASE/far-input.log" | wc -c | tr -d ' ')" -ge 10 ]]; }
wait_until "10 more keys at the far program" ten_b
sleep 1
[[ "$(far_size "$FAR_ADDR")" == "90x25" ]] || fail "V14: typing after the far resize moved the far terminal to $(far_size "$FAR_ADDR")"
python3 -I - "$FRAMES" "$mark" <<'PY' || fail "V14: the view sent a frame it must not"
import sys
rows = [line.split() for line in open(sys.argv[1]).read().splitlines()[int(sys.argv[2]):]]
streams = [r for r in rows if r[1] != "0"]
kinds = {}
for r in streams:
    kinds[r[2]] = kinds.get(r[2], 0) + 1
bad = {k: n for k, n in kinds.items() if k not in ("1", "3", "6")}
resizes = [r for r in streams if r[2] == "3"]
print(f"V14 frames from the view on streams: {kinds} (0 DATA, 1 CREDIT, 3 RESIZE, 4 FOCUS, 5 FACTS, 6 AUTOMATIC_INPUT)")
for r in resizes:
    cols = int.from_bytes(bytes.fromhex(r[4])[0:2], "little"); rows_ = int.from_bytes(bytes.fromhex(r[4])[2:4], "little")
    print(f"  RESIZE {cols}x{rows_}")
ok = not bad and "6" in kinds and len(resizes) == 1 and resizes[0][4].startswith("5a001900")
raise SystemExit(0 if ok else 1)
PY
echo "ok: V14 only AUTOMATIC_INPUT, CREDIT and one RESIZE (90x25) went from the view; the far terminal stayed 90x25"
rm -f "$BASE/hold"
wait "$VIEWER_PID" 2>/dev/null || true

# --- V30: an idle view sends nothing but CREDIT ---------------------------
# Past the view's typing window (30 s), so no machine read is due.
sleep 33
# The far viewer that took 90x25 has left: core applied the view's recorded
# size, which is the last far size the view echoed, not its attach size.
FAR_SIZE1="$(far_size "$FAR_ADDR")"
[[ "$FAR_SIZE1" == "90x25" ]] || fail "V30: with the far viewer gone the far terminal is $FAR_SIZE1, not the last far size 90x25"
mark="$(frame_count)"
sleep "$IDLE_SECS"
python3 -I - "$FRAMES" "$mark" "$IDLE_SECS" <<'PY' || fail "V30: an idle view sent frames"
import sys
rows = [line.split() for line in open(sys.argv[1]).read().splitlines()[int(sys.argv[2]):]]
kinds = {}
for r in rows:
    kinds[(r[1] == "0", r[2])] = kinds.get((r[1] == "0", r[2]), 0) + 1
other = [r for r in rows if r[2] != "1"]
print(f"V30: {len(rows)} frames from the idle view in {sys.argv[3]} s, {len(other)} not CREDIT")
raise SystemExit(0 if not other else 1)
PY
# The far herdr drops (killed, so its viewer just goes): core keeps the far
# size, not the view's own pane size (R8).
kill -9 "$FAR_PID"
sleep 1.5
[[ "$(far_size "$FAR_ADDR")" == "$FAR_SIZE1" ]] || fail "V30: after the far herdr dropped the far terminal is $(far_size "$FAR_ADDR"), not $FAR_SIZE1"
[[ "$(far_pid "$FAR_ADDR")" == "$FAR_PROGRAM" ]] || fail "the far program ended with its herdr"
echo "ok: V30 idle view sent only CREDIT; with the far herdr gone the far terminal kept $FAR_SIZE1"
far "$HERDR_BIN" --session far server >/dev/null 2>&1 &
FAR_PID=$!
wait_until "far herdr again" far_up

# --- V17: closing in the view is a detach ---------------------------------
tm send-keys -t view C-b x
sleep 0.5
tm send-keys -t view y
sleep 2
[[ "$(far_pid "$FAR_ADDR")" == "$FAR_PROGRAM" ]] || fail "V17: closing the view's pane ended the far program"
api_far "{\"id\":\"v\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"$PANE\"}}" >/dev/null \
  || fail "V17: the far pane is gone"
tm kill-session -t view
kill "$VIEW_PID"
view_gone() { ! kill -0 "$VIEW_PID" 2>/dev/null; }
wait_until "the view server to stop" view_gone
VIEW_PID=""
sleep 1
[[ "$(far_pid "$FAR_ADDR")" == "$FAR_PROGRAM" ]] || fail "V17: stopping the view ended the far program"
api_far "{\"id\":\"v\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"$PANE\"}}" >/dev/null \
  || fail "V17: the far pane is gone after the view stopped"
echo "ok: V17 closing a pane in the view and stopping the view left far program $FAR_PROGRAM and pane $PANE"

echo "core view live test: all checks passed"
