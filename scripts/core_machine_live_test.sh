#!/usr/bin/env bash
# Core machine checks (openspec/changes/core-phase5). Two scratch cores on this
# Mac stand in for two Macs: the far one is reached by `hc machine add --via`
# with a local relay script, never ssh. A user's herdr and cores are never
# reached; every core call carries a scratch HC_SESSION.
#
# V26 (PR A0, F1): a far terminal that carries the same labels as one of our
# panes, and a pid that is a live process here, is never adopted, listed as an
# orphan, killed or signalled by the local server, across a live handoff, a
# restart and closing the local pane.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERDR_BIN="${HERDR_BIN:-$ROOT/target/debug/herdr}"
BASE="$(mktemp -d /tmp/hcm.XXXXXX)"
CONFIG_HOME="$BASE/c"
RUNTIME_DIR="$BASE/r"
STATE_DIR="$BASE/s"
FAR_RUNTIME_DIR="$BASE/fr"
FAR_STATE_DIR="$BASE/fs"
SESSION="v26"
CORE_SESSION="herdrdev-$SESSION"
FAR_SESSION="herdrdev-$SESSION-far"
SOCKET="$CONFIG_HOME/herdr-dev/sessions/$SESSION/herdr.sock"

case "$BASE" in
  /tmp/hcm.*) ;;
  *) echo "refusing scratch folder $BASE" >&2; exit 1 ;;
esac

run_herdr() {
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
    -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    SHELL=/bin/sh PATH="$BASE/bin:$PATH" \
    "$HERDR_BIN" --session "$SESSION" "$@"
}

hc_scratch() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HC_SESSION="$CORE_SESSION" hc "$@"
}

hc_far() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$FAR_RUNTIME_DIR" XDG_STATE_HOME="$FAR_STATE_DIR" \
    HC_SESSION="$FAR_SESSION" hc "$@"
}

cleanup() {
  set +e
  run_herdr server stop >/dev/null 2>&1
  # Only a running one: any hc call would start the session.
  [[ -S "$RUNTIME_DIR/herdr-core/$CORE_SESSION.sock" ]] && hc_scratch server stop >/dev/null 2>&1
  [[ -S "$FAR_RUNTIME_DIR/herdr-core/$FAR_SESSION.sock" ]] && hc_far server stop >/dev/null 2>&1
  [[ -s "$BASE/far.pid" ]] && kill "$(cat "$BASE/far.pid")" 2>/dev/null
  for pid in $(jobs -p); do kill "$pid" 2>/dev/null; done
  rm -rf "$BASE"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  echo "--- server log tail ---" >&2
  tail -n 40 "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.log 2>/dev/null >&2 || true
  exit 1
}

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
  api "$(python3 -I -c 'import json,sys; print(json.dumps({"id":"v26","method":"pane.send_input","params":{"pane_id":sys.argv[1],"text":sys.argv[2],"keys":["Enter"]}}))' "$1" "$2")" >/dev/null
}

wait_until() {
  local what="$1"
  shift
  for _ in $(seq 1 200); do
    if "$@"; then return 0; fi
    sleep 0.05
  done
  fail "timed out waiting for: $what"
}

file_has_line() { [[ -s "$1" ]] && [[ "$(tail -c1 "$1" | od -An -c | tr -d ' ')" == '\n' ]]; }
alive() { kill -0 "$1" 2>/dev/null; }
socket_up() { [[ -S "$SOCKET" ]] && api '{"id":"v26","method":"ping","params":{}}' >/dev/null 2>&1; }

# The addresses of the local core's live terminals, as it lists them.
listed_addresses() {
  hc_scratch ls --json --label "herdr.session=$CORE_SESSION" \
    | python3 -I -c 'import json,sys; print(" ".join(sorted(t["address"] for t in json.load(sys.stdin)["terminals"] if t["state"] == "live")))'
}

echo "herdr: $HERDR_BIN"
echo "scratch: $BASE"
mkdir -p "$CONFIG_HOME/herdr-dev" "$RUNTIME_DIR" "$STATE_DIR" "$FAR_RUNTIME_DIR" "$FAR_STATE_DIR" "$BASE/bin"
printf 'onboarding = false\n\n[experimental]\nruntime = "core"\n' > "$CONFIG_HOME/herdr-dev/config.toml"

# The relay: core hands it one shell line, which runs against the far core.
cat > "$BASE/via.sh" <<VIA
#!/bin/sh
for line; do :; done
exec env -u HC_SESSION XDG_RUNTIME_DIR="$FAR_RUNTIME_DIR" XDG_STATE_HOME="$FAR_STATE_DIR" sh -c "\$line"
VIA
chmod +x "$BASE/via.sh"

run_herdr server >/dev/null 2>&1 &
wait_until "server socket" socket_up
A="$(api "{\"id\":\"v26\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])')"
send_line "$A" "echo \$\$ > $BASE/a.pid"
wait_until "pane A pid" file_has_line "$BASE/a.pid"
A_PID="$(cat "$BASE/a.pid")"
LOCAL="$(listed_addresses)"
[[ "$LOCAL" =~ ^t[0-9]+$ ]] || fail "expected one local core terminal for pane A: $LOCAL"

# The machine, added by hand: from now the local core lists its terminals.
hc_scratch machine add --json --name loop --session "$FAR_SESSION" --via "$BASE/via.sh" >/dev/null \
  || fail "machine add failed"
# A far terminal with pane A's labels, and a pid that is a live process here:
# the far shell's own pid, which this test also holds a process for.
hc_scratch run --bg --no-parent --no-notify --on loop \
  --label "herdr.session=$CORE_SESSION" --label "herdr.pane_id=$A" \
  sh -c "echo \$\$ > $BASE/far.pid; exec sleep 600" >/dev/null || fail "far run failed"
wait_until "far program pid" file_has_line "$BASE/far.pid"
FAR_PID="$(cat "$BASE/far.pid")"
alive "$FAR_PID" || fail "the far program is not running"
BOTH="$(listed_addresses)"
[[ "$BOTH" == "loop/t1 $LOCAL" ]] || fail "the local core should list $LOCAL and loop/t1: $BOTH"
echo "ok: the local core lists $BOTH; loop/t1 has pane A's labels, pid $FAR_PID is live here"

far_untouched() {
  alive "$FAR_PID" || fail "$1 ended the far program (pid $FAR_PID)"
  [[ "$(listed_addresses)" == "$BOTH" ]] || fail "$1 changed the terminals: $(listed_addresses)"
  if grep -h "loop/t" "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.log 2>/dev/null \
    | command grep -Ei "adopt|attach|orphan|has no pane|kill|sweep|signal"; then
    fail "$1 mentioned the far terminal in the server log"
  fi
}

# 1. A live handoff: the restore search runs; pane A keeps its own terminal.
run_herdr server live-handoff --import-exe "$HERDR_BIN" >/dev/null || fail "live handoff did not complete"
wait_until "server after handoff" socket_up
alive "$A_PID" || fail "pane A's program ended in the handoff"
far_untouched "the live handoff"
echo "ok: a live handoff left loop/t1 alone"

# 2. A restart: the orphan scan and the label search run again.
run_herdr server restart --yes >"$BASE/restart.out" 2>&1 || fail "restart failed: $(tail -n 5 "$BASE/restart.out")"
wait_until "server after restart" socket_up
alive "$A_PID" || fail "pane A's program ended in the restart"
far_untouched "the restart"
send_line "$A" "echo after-restart > $BASE/a.restart"
wait_until "input to pane A after the restart" file_has_line "$BASE/a.restart"
echo "ok: a restart reattached pane A to $LOCAL and left loop/t1 alone"

# 3. Closing the local pane ends the local terminal and sweeps its own session
#    only: the far program and its record stay.
api "{\"id\":\"v26\",\"method\":\"pane.close\",\"params\":{\"pane_id\":\"$A\"}}" >/dev/null || fail "pane.close failed"
gone_local() { [[ "$(listed_addresses)" == "loop/t1" ]]; }
wait_until "the local terminal ended" gone_local
alive "$FAR_PID" || fail "closing pane A ended the far program (pid $FAR_PID)"
echo "ok: closing pane A ended $LOCAL only; the far program (pid $FAR_PID) still runs"
echo "core machine live test: all checks passed"
