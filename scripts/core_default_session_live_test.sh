#!/usr/bin/env bash
# Release-build herdr-core checks for the default herdr session
# (openspec/changes/core-phase3, PR C). A release build's default herdr
# session puts new panes in core session `default` and keeps the label
# `herdr`, so only a release build can show it. V17: a new pane goes to
# `default`, a pane whose saved link names the legacy `herdr` core is
# re-adopted there, and a terminal in `default` without our label (the
# phone's) is never touched. V19: a PTY pane, a pane in `herdr` and one in
# `default` through a live handoff, a restart and a stop. V21, when
# HERDR_R0_BIN (the PR B beta) and HERDR_202_BIN (beta 202) name binaries:
# a live handoff to the PR B beta re-adopts a `default` pane with its pid; a
# handoff to beta 202 starts that pane fresh and leaves its terminal running;
# the handoff back to this build names that terminal as having no pane, and
# re-adopts a pane beta 202 started in `herdr` (V17 with a real phase 2
# build).
#
# Everything lives under one scratch folder: herdr's config and sockets, and
# the scratch herdr-core sessions `default` and `herdr` (core follows
# XDG_RUNTIME_DIR and XDG_STATE_HOME). A user's herdr and cores are never
# reached.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERDR_BIN="${HERDR_BIN:-$ROOT/target/release/herdr}"
BASE="$(mktemp -d /tmp/hcd.XXXXXX)"
CONFIG_HOME="$BASE/c"
RUNTIME_DIR="$BASE/r"
STATE_DIR="$BASE/s"
DATA="$CONFIG_HOME/herdr"
SOCKET="$DATA/herdr.sock"

case "$BASE" in
  /tmp/hcd.*) ;;
  *) echo "refusing scratch folder $BASE" >&2; exit 1 ;;
esac
for dir in "$CONFIG_HOME" "$RUNTIME_DIR" "$STATE_DIR"; do
  case "$dir" in
    "$BASE"/*) ;;
    *) echo "refusing $dir outside $BASE: it could reach a real core session default" >&2; exit 1 ;;
  esac
done

run_bin() {
  local bin="$1"
  shift
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
    -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    SHELL=/bin/sh PATH="$BASE/bin:$PATH" \
    "$bin" "$@"
}
run_herdr() { run_bin "$HERDR_BIN" "$@"; }

core_socket() { echo "$RUNTIME_DIR/herdr-core/$1.sock"; }
# hc in a scratch core session. Any hc call starts a session that is not
# running, so checks of one herdr must not start look at its socket first.
hc_in() {
  local session="$1"
  shift
  env -u HC_SOCKET XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HC_SESSION="$session" hc "$@"
}

cleanup() {
  set +e
  run_herdr server stop >/dev/null 2>&1
  for session in default herdr; do
    [[ -S "$(core_socket "$session")" ]] && hc_in "$session" server stop >/dev/null 2>&1
  done
  for pid in $(jobs -p); do kill "$pid" 2>/dev/null; done
  rm -rf "$BASE"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  echo "--- server log tail ---" >&2
  tail -n 40 "$DATA"/*.log 2>/dev/null >&2 || true
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
  api "$(python3 -I -c 'import json,sys; print(json.dumps({"id":"c","method":"pane.send_input","params":{"pane_id":sys.argv[1],"text":sys.argv[2],"keys":["Enter"]}}))' "$1" "$2")" >/dev/null
}

wait_for() {
  local tries="$1" what="$2"
  shift 2
  for _ in $(seq 1 "$tries"); do
    if "$@"; then return 0; fi
    sleep 0.05
  done
  fail "timed out waiting for: $what"
}
wait_until() { wait_for 200 "$@"; }
wait_long() { wait_for 1200 "$@"; }

file_has_line() { [[ -s "$1" ]] && [[ "$(tail -c1 "$1" | od -An -c | tr -d ' ')" == '\n' ]]; }
alive() { kill -0 "$1" 2>/dev/null; }
gone() { ! kill -0 "$1" 2>/dev/null; }
socket_up() { [[ -S "$SOCKET" ]] && api '{"id":"c","method":"ping","params":{}}' >/dev/null 2>&1; }

# One line per live terminal labelled ours in a running core session:
# address, pid, pane label.
terminals() {
  local session="$1" address
  [[ -S "$(core_socket "$session")" ]] || return 0
  for address in $(hc_in "$session" ls --json --label "herdr.session=herdr" 2>/dev/null \
    | python3 -I -c 'import json,sys; [print(t["address"]) for t in json.load(sys.stdin)["terminals"] if t["state"] == "live"]'); do
    hc_in "$session" get "$address" --json \
      | python3 -I -c 'import json,sys; t=json.load(sys.stdin); print(t["address"], t.get("pid") or 0, (t.get("labels") or {}).get("herdr.pane_id","-"))'
  done
}
field_of() { terminals "$1" | awk -v pane="$2" -v n="$3" '$3 == pane { print $n }'; }
pid_in() { field_of "$1" "$2" 2; }
address_in() { field_of "$1" "$2" 1; }

write_config() {
  printf 'onboarding = false\n\n[update]\nversion_check = false\nmanifest_check = false\n\n[experimental]\nruntime = "%s"\n' \
    "$1" > "$DATA/config.toml"
}

pane_get() { api "{\"id\":\"c\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"$1\"}}"; }
pane_field() {
  pane_get "$1" | python3 -I -c 'import json,sys; r=json.load(sys.stdin); p=r.get("pane", r); print(p.get(sys.argv[1]) or False)' "$2"
}
is_running() { [[ "$(pane_field "$1" suspended)" == False && "$(pane_field "$1" core_waiting)" == False ]]; }
split() {
  api "{\"id\":\"c\",\"method\":\"pane.split\",\"params\":{\"target_pane_id\":\"$1\",\"direction\":\"$2\"}}" \
    | python3 -I -c 'import json,sys; r=json.load(sys.stdin); print((r.get("pane") or r)["pane_id"])'
}
new_workspace() {
  api "{\"id\":\"c\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
    | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])'
}
# The pid of the shell in pane $1, as it prints it into file $2.
shell_pid() {
  rm -f "$2"
  send_line "$1" "echo \$\$ > $2"
  wait_until "pane $1's pid" file_has_line "$2"
  cat "$2"
}
server_pid() {
  python3 -I - "$SOCKET" <<'PY2'
import socket, struct, sys
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(sys.argv[1])
if sys.platform == "darwin":
    print(struct.unpack("i", client.getsockopt(0, 2, 4))[0])  # SOL_LOCAL, LOCAL_PEERPID
else:
    print(struct.unpack("3i", client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0])
PY2
}
server_version() { run_herdr status server --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("version") or "-")'; }
core_server_pid() { hc_in "$1" server status --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["pid"])'; }
no_pane_logged() {
  grep -h "has no pane" "$DATA"/*.log 2>/dev/null | grep "session=$1" | grep -q "address=$2"
}
handoff_to() {
  run_herdr server live-handoff --import-exe "$1" >"$BASE/handoff.out" 2>&1 \
    || fail "live handoff to $1 did not complete: $(tail -n 5 "$BASE/handoff.out")"
  wait_until "server after the handoff to $1" socket_up
}

# Points the saved link of the pane on $2 in core session $1 at $4 in core
# session $3, as a link from before the pid fields; prints the count.
relink() {
  python3 -I - "$DATA" "$@" <<'PY2'
import json, pathlib, sys
folder, session, address, other, other_address = sys.argv[1:]
def walk(node):
    count = 0
    if isinstance(node, dict):
        link = node.get("core_terminal")
        if isinstance(link, dict) and link.get("session") == session and link.get("address") == address:
            node["core_terminal"] = {"session": other, "address": other_address}
            count += 1
        for value in node.values():
            count += walk(value)
    elif isinstance(node, list):
        for value in node:
            count += walk(value)
    return count
total = 0
for path in pathlib.Path(folder).glob("*.json"):
    try:
        data = json.loads(path.read_text())
    except ValueError:
        continue
    count = walk(data)
    if count:
        path.write_text(json.dumps(data))
        total += count
print(total)
PY2
}

echo "herdr: $HERDR_BIN"
echo "scratch: $BASE"
mkdir -p "$DATA" "$RUNTIME_DIR" "$STATE_DIR" "$BASE/bin"
chmod 700 "$RUNTIME_DIR" "$STATE_DIR"

# 1. V17 (D5, D6): a PTY pane, then a core pane, in the default herdr session
#    of a release build.
write_config pty
run_herdr server >/dev/null 2>&1 &
wait_until "server socket at $SOCKET (a release build's default session)" socket_up
A="$(new_workspace)"
A_PID="$(shell_pid "$A" "$BASE/a.pid")"
write_config core
run_herdr server reload-config >/dev/null
B="$(split "$A" right)"
B_PID="$(shell_pid "$B" "$BASE/b.pid")"
[[ "$(pid_in default "$B")" == "$B_PID" ]] \
  || fail "new pane B ($B_PID) is not in core session default with the label herdr: $(terminals default)"
[[ -S "$(core_socket herdr)" ]] && fail "herdr started the legacy core session herdr"
echo "ok: new pane B ($B, pid $B_PID) runs in core session default, labelled herdr.session=herdr"

# The phone's terminal: in default, without our label.
FOREIGN="$(hc_in default run --bg --json --no-parent -- /bin/sh -c 'echo $$ > "$0"; exec sleep 600' "$BASE/foreign.pid" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["address"])')"
wait_until "the foreign terminal's pid" file_has_line "$BASE/foreign.pid"
FOREIGN_PID="$(cat "$BASE/foreign.pid")"
DEFAULT_CORE="$(core_server_pid default)"

# A pane from before the move: with our server killed (core programs live
# on), pane G's saved link is pointed at a program labelled for G in the
# legacy core session herdr, as phase 2 left them.
G="$(split "$B" down)"
G_NEW_PID="$(shell_pid "$G" "$BASE/g.pid")"
G_DEFAULT="$(address_in default "$G")"
[[ -n "$G_DEFAULT" ]] || fail "pane G is not in default: $(terminals default)"
saved_with_g() { grep -q "\"$G_DEFAULT\"" "$DATA"/*.json 2>/dev/null; }
wait_until "G's link saved" saved_with_g
G_HERDR="$(hc_in herdr run --bg --json --no-parent --label "herdr.session=herdr" --label "herdr.pane_id=$G" \
  -- /bin/sh -c 'echo $$ > "$0"; exec /bin/sh' "$BASE/g.herdr.pid" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["address"])')"
wait_until "G's program in herdr" file_has_line "$BASE/g.herdr.pid"
G_PID="$(cat "$BASE/g.herdr.pid")"
HERDR_CORE="$(core_server_pid herdr)"
HERDR_PID="$(server_pid)"
kill -KILL "$HERDR_PID"
wait_until "our server gone" gone "$HERDR_PID"
hc_in default kill "$G_DEFAULT" >/dev/null
wait_until "G's shell in default gone" gone "$G_NEW_PID"
[[ "$(relink default "$G_DEFAULT" herdr "$G_HERDR")" -ge 1 ]] || fail "no saved link of G to point at herdr"
run_herdr server >/dev/null 2>&1 &
wait_until "server after the kill" socket_up
wait_long "pane G attached in herdr" is_running "$G"
[[ "$(shell_pid "$G" "$BASE/g.after")" == "$G_PID" ]] || fail "G does not run its program in herdr ($G_PID)"
[[ -z "$(pid_in default "$G")" ]] || fail "a program was started for G in default: $(terminals default)"
[[ "$(shell_pid "$B" "$BASE/b.after")" == "$B_PID" ]] || fail "B lost its program in default"
A_PID="$(shell_pid "$A" "$BASE/a2.pid")"
echo "ok: pane G re-adopted from the legacy core herdr (pid $G_PID), nothing started for it in default"

foreign_untouched() {
  alive "$FOREIGN_PID" || fail "the unlabelled terminal in default ended ($1)"
  [[ "$(hc_in default get "$FOREIGN" --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["state"])')" == live ]] \
    || fail "the unlabelled terminal in default is no longer live ($1)"
  if grep -h "has no pane" "$DATA"/*.log 2>/dev/null | grep -q "address=$FOREIGN "; then
    fail "the unlabelled terminal $FOREIGN was listed as ours ($1)"
  fi
}
foreign_untouched "after the restore"
echo "ok: the unlabelled terminal $FOREIGN in default is untouched"

# 2. V19 (core-backend V10): live handoff, then restart, then stop.
handoff_to "$HERDR_BIN"
for pid in "$A_PID" "$B_PID" "$G_PID"; do alive "$pid" || fail "program $pid ended in the handoff"; done
[[ "$(shell_pid "$A" "$BASE/a.handoff")" == "$A_PID" ]] || fail "PTY pane A changed in the handoff"
[[ "$(shell_pid "$B" "$BASE/b.handoff")" == "$B_PID" ]] || fail "B changed in the handoff"
[[ "$(shell_pid "$G" "$BASE/g.handoff")" == "$G_PID" ]] || fail "G changed in the handoff"
echo "ok: live handoff kept PTY pane A, B in default and G in herdr, each with its pid"

HERDR_PID="$(server_pid)"
run_herdr server restart --yes >"$BASE/restart.out" 2>&1 &
wait_long "the old server gone in the restart" gone "$HERDR_PID"
wait_until "server after the restart" socket_up
wait_until "PTY program ended by the restart" gone "$A_PID"
wait_long "pane B back after the restart" is_running "$B"
wait_long "pane G back after the restart" is_running "$G"
[[ "$(shell_pid "$B" "$BASE/b.restart")" == "$B_PID" ]] || fail "B changed in the restart"
[[ "$(shell_pid "$G" "$BASE/g.restart")" == "$G_PID" ]] || fail "G changed in the restart"
[[ "$(core_server_pid default)" == "$DEFAULT_CORE" && "$(core_server_pid herdr)" == "$HERDR_CORE" ]] \
  || fail "a core changed in the restart"
foreign_untouched "after the restart"
echo "ok: restart kept B in default and G in herdr; A restarted on the PTY"

run_herdr server stop >/dev/null
wait_until "B's program ended by the stop" gone "$B_PID"
wait_until "G's program in herdr ended by the stop" gone "$G_PID"
alive "$DEFAULT_CORE" || fail "core session default ended with our server"
alive "$HERDR_CORE" || fail "core session herdr ended with our server"
alive "$FOREIGN_PID" || fail "the unlabelled terminal in default ended with our server"
echo "ok: stop ended B and G in both sessions, and left both cores and the unlabelled terminal running"

# 3. V21 (D10, D14): rollback by live handoff to the PR B beta, then to beta 202.
if [[ -z "${HERDR_R0_BIN:-}" || -z "${HERDR_202_BIN:-}" ]]; then
  echo "skip: V21 rollback (set HERDR_R0_BIN to the PR B beta and HERDR_202_BIN to beta 202)"
  echo "PASS: core default session live test (core-phase3 V17, V19)"
  exit 0
fi
run_herdr server >/dev/null 2>&1 &
wait_until "server for V21" socket_up
P="$(new_workspace)"
P_PID="$(shell_pid "$P" "$BASE/p.pid")"
P_ADDR="$(address_in default "$P")"
[[ -n "$P_ADDR" && "$(pid_in default "$P")" == "$P_PID" ]] || fail "pane P is not in default: $(terminals default)"
OURS="$(server_version)"

handoff_to "$HERDR_R0_BIN"
R0="$(server_version)"
[[ "$R0" != "$OURS" ]] || fail "the handoff to $HERDR_R0_BIN still runs $OURS"
wait_long "pane P re-adopted by $R0" is_running "$P"
[[ "$(shell_pid "$P" "$BASE/p.r0")" == "$P_PID" ]] || fail "$R0 did not re-adopt P's program in default"
[[ "$(address_in default "$P")" == "$P_ADDR" ]] || fail "P's terminal changed under $R0: $(terminals default)"
echo "ok: $R0 re-adopted P in default ($P_ADDR, pid $P_PID)"

handoff_to "$HERDR_202_BIN"
OLD="$(server_version)"
wait_long "pane P running under $OLD" is_running "$P"
P_FRESH="$(shell_pid "$P" "$BASE/p.202")"
[[ "$P_FRESH" != "$P_PID" ]] || fail "$OLD re-adopted P; it was expected to start it fresh"
alive "$P_PID" || fail "P's program in default ended under $OLD"
echo "ok: $OLD started P fresh (pid $P_FRESH); its terminal $P_ADDR in default keeps running (pid $P_PID)"
# V17 with a real phase 2 build: a new pane there goes to the legacy core.
Q="$(split "$P" down)"
Q_PID="$(shell_pid "$Q" "$BASE/q.pid")"
[[ "$(pid_in herdr "$Q")" == "$Q_PID" ]] || fail "$OLD did not put new pane Q in core session herdr: $(terminals herdr)"

handoff_to "$HERDR_BIN"
wait_long "the orphan in default named by $OURS" no_pane_logged default "$P_ADDR"
alive "$P_PID" || fail "P's old program ended after the handoff back"
foreign_untouched "after the rollback"
echo "ok: back on $OURS, the terminal $P_ADDR in default is named as having no pane, and left running"
wait_long "pane Q re-adopted from herdr" is_running "$Q"
[[ "$(shell_pid "$Q" "$BASE/q.back")" == "$Q_PID" ]] || fail "$OURS did not re-adopt Q's program in herdr"
R="$(split "$Q" down)"
R_PID="$(shell_pid "$R" "$BASE/r.pid")"
[[ "$(pid_in default "$R")" == "$R_PID" ]] || fail "new pane R is not in default: $(terminals default)"
echo "ok: pane Q, started by $OLD in herdr, re-adopted with its pid; new pane R runs in default"
echo "PASS: core default session live test (core-phase3 V17, V19, V21)"
