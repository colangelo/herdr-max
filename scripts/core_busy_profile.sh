#!/usr/bin/env bash
# V12 (openspec/changes/core-backend): the cost of N busy panes on the PTY and
# on herdr-core, measured on one throwaway herdr. Each pane prints one 90-byte
# line twice a second; the script reports the CPU time used in WINDOW seconds
# busy and WINDOW seconds idle by herdr's own processes (server and pane
# anchor) and, on core, the core server and its holders, plus their memory
# footprint. The panes' own programs are not counted.
#
# usage: scripts/core_busy_profile.sh pty|core   (HERDR_BIN: a release build)
# options: V12_ONE_PROCESS=1 (one printer per pane, no child per line),
#          V12_TOP=<file> (kernel counters), V12_SAMPLE=<file> (a profile)
set -euo pipefail

RUNTIME="${1:?usage: core_busy_profile.sh pty|core}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERDR_BIN="${HERDR_BIN:-$ROOT/target/release/herdr}"
PANES="${PANES:-30}"
WINDOW="${WINDOW:-30}"
BASE="$(mktemp -d /tmp/hcp.XXXXXX)"
CONFIG_HOME="$BASE/c"
RUNTIME_DIR="$BASE/r"
STATE_DIR="$BASE/s"
SESSION="v12"

case "$BASE" in
  /tmp/hcp.*) ;;
  *) echo "refusing scratch folder $BASE" >&2; exit 1 ;;
esac

# A debug build keeps its state under herdr-dev and names its core herdrdev-*.
case "$HERDR_BIN" in
  */target/debug/*) DEV="-dev" CORE_PREFIX="herdrdev" ;;
  *) DEV="" CORE_PREFIX="herdr" ;;
esac
SOCKET="$CONFIG_HOME/herdr$DEV/sessions/$SESSION/herdr.sock"
CORE_SESSION="$CORE_PREFIX-$SESSION"

run_herdr() {
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
    -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    SHELL=/bin/sh HERDR_NO_MACHINE_LEASES=1 \
    "$HERDR_BIN" --session "$SESSION" "$@"
}

hc_scratch() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HC_SESSION="$CORE_SESSION" hc "$@"
}

cleanup() {
  set +e
  run_herdr server stop >/dev/null 2>&1
  hc_scratch server stop >/dev/null 2>&1
  for pid in $(jobs -p); do kill "$pid" 2>/dev/null; done
  rm -rf "$BASE"
}
trap cleanup EXIT

api() {
  python3 -I - "$SOCKET" "$1" <<'PY'
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

send_line() {
  api "$(python3 -I -c 'import json,sys; print(json.dumps({"id":"v12","method":"pane.send_input","params":{"pane_id":sys.argv[1],"text":sys.argv[2],"keys":["Enter"]}}))' "$1" "$2")" >/dev/null
}

socket_up() { [[ -S "$SOCKET" ]] && api '{"id":"v12","method":"ping","params":{}}' >/dev/null 2>&1; }

# CPU seconds (user + system) a process has used so far.
cpu_seconds() {
  ps -o time= -p "$1" 2>/dev/null | python3 -I -c '
import sys
text = sys.stdin.read().strip()
if not text:
    print(0); raise SystemExit
parts = [float(p) for p in text.replace("-", ":").split(":")]
total = 0.0
for p in parts:
    total = total * 60 + p
print(total)'
}

footprint_mb() {
  footprint "$1" 2>/dev/null | python3 -I -c '
import re, sys
m = re.search(r"Footprint:\s*([\d.]+)\s*([KMG]B)", sys.stdin.read())
if not m:
    print(0); raise SystemExit
v, unit = float(m.group(1)), m.group(2)
print(v / 1024 if unit == "KB" else v * 1024 if unit == "GB" else v)'
}

measured_pids() {
  local pids=("$SERVER_PID")
  local anchor
  anchor="$(pgrep -f "$CONFIG_HOME/herdr$DEV/pane-anchor/" || true)"
  [[ -n "$anchor" ]] && pids+=($anchor)
  if [[ "$RUNTIME" == core ]]; then
    pids+=($(hc_scratch server status --json | python3 -I -c '
import json, sys
s = json.load(sys.stdin)
print(s["pid"], *[h["pid"] for h in s.get("holders", [])])'))
  fi
  echo "${pids[@]}"
}

total_cpu() {
  local sum=0 pid
  for pid in $(measured_pids); do
    sum="$(python3 -I -c 'import sys; print(float(sys.argv[1]) + float(sys.argv[2]))' "$sum" "$(cpu_seconds "$pid")")"
  done
  echo "$sum"
}

# CPU per measured process over one window: "total pid:seconds ...".
window_cpu() {
  local pids before=() after=() pid i
  pids=($(measured_pids))
  for pid in "${pids[@]}"; do before+=("$(cpu_seconds "$pid")"); done
  sleep "$WINDOW"
  for pid in "${pids[@]}"; do after+=("$(cpu_seconds "$pid")"); done
  python3 -I -c '
import sys
n = len(sys.argv) // 3
pids, before, after = sys.argv[1:1+n], sys.argv[1+n:1+2*n], sys.argv[1+2*n:]
parts = [(p, float(a) - float(b)) for p, b, a in zip(pids, before, after)]
print(f"{sum(d for _, d in parts):.2f}", " ".join(f"{p}:{d:.2f}" for p, d in parts))' \
    "${pids[@]}" "${before[@]}" "${after[@]}"
}

mkdir -p "$CONFIG_HOME/herdr$DEV" "$RUNTIME_DIR" "$STATE_DIR"
printf 'onboarding = false\n\n[experimental]\nruntime = "%s"\n' "$RUNTIME" > "$CONFIG_HOME/herdr$DEV/config.toml"
run_herdr server >/dev/null 2>&1 &
for _ in $(seq 1 200); do socket_up && break; sleep 0.05; done
socket_up || { echo "server did not start" >&2; exit 1; }
# `$!` is the subshell running run_herdr; the server is the herdr process.
SERVER_PID="$(pgrep -f "^$HERDR_BIN --session $SESSION server\$")"
[[ -n "$SERVER_PID" ]] || { echo "server process not found" >&2; exit 1; }

# N panes, one per workspace so each has the whole headless size.
PANE_IDS=()
for _ in $(seq 1 "$PANES"); do
  PANE_IDS+=("$(api "{\"id\":\"v12\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\"}}" \
    | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])')")
done
for pane in "${PANE_IDS[@]}"; do
  if [[ -n "${V12_ONE_PROCESS:-}" ]]; then
    # One long-running printer: no new process per line (no sleep child).
    send_line "$pane" "touch $BASE/busy; perl -e 'use Time::HiRes qw(sleep); \$|=1; while (-e \$ARGV[0]) { printf(\"%090d\\n\", 0); sleep 0.5 }' $BASE/busy"
  else
    send_line "$pane" "touch $BASE/busy; while [ -e $BASE/busy ]; do printf '%090d\\n' 0; sleep 0.5; done"
  fi
done
if [[ "$RUNTIME" == core ]]; then
  live="$(hc_scratch ls --json --label "herdr.session=$CORE_SESSION" \
    | python3 -I -c 'import json,sys; print(sum(t["state"] == "live" for t in json.load(sys.stdin)["terminals"]))')"
  [[ "$live" == "$PANES" ]] || { echo "expected $PANES core terminals, found $live" >&2; exit 1; }
fi
sleep 5

# V12_SAMPLE=<file>: also take a macOS `sample` (V12_SAMPLE_S seconds, default 10) of the server inside
# the busy window (for a profile only; it slows the server, so do not report
# that run's CPU).
if [[ -n "${V12_SAMPLE:-}" ]]; then
  (sleep 5; sample "$SERVER_PID" ${V12_SAMPLE_S:-10} -file "$V12_SAMPLE" >/dev/null 2>&1) &
fi
# V12_TOP=<file>: context switches and system calls of the server over 20 s
# of the busy window (`top`), to tell what kind of kernel work grows.
if [[ -n "${V12_TOP:-}" ]]; then
  (sleep 3; top -l 2 -s 20 -pid "$SERVER_PID" -stats pid,cpu,csw,sysbsd,sysmach,msgsent,msgrecv,th > "$V12_TOP" 2>&1) &
fi
read -r BUSY BUSY_BY <<<"$(window_cpu)"
rm -f "$BASE/busy"
sleep 3
read -r IDLE IDLE_BY <<<"$(window_cpu)"
MEM=0
for pid in $(measured_pids); do
  MEM="$(python3 -I -c 'import sys; print(f"{float(sys.argv[1]) + float(sys.argv[2]):.1f}")' "$MEM" "$(footprint_mb "$pid")")"
done
if [[ -n "${V12_DEBUG:-}" ]]; then
  ps -o pid,ppid,time,rss,command -p "$(measured_pids | tr ' ' ',')" >&2
  ls "$BASE" >&2
fi
echo "runtime=$RUNTIME panes=$PANES window=${WINDOW}s busy_cpu=${BUSY}s idle_cpu=${IDLE}s footprint=${MEM}MB"
echo "  busy by pid (server anchor [core holder]): $BUSY_BY"
echo "  idle by pid: $IDLE_BY"
