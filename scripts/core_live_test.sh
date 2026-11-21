#!/usr/bin/env bash
# Server-level herdr-core checks (openspec/changes/core-backend): a throwaway
# herdr with one PTY pane and one herdr-core pane. V15: the core it starts is
# not tied to it; V11: a hook in the core pane reports through our API, and
# with `core_status_mirror` on, the status reaches core's record
# (openspec/changes/core-status-mirror); with `core_spaces_doc` on, the layout
# merges into core's `herdr-app` document beside another client's space
# (openspec/changes/core-spaces-doc); V10:
# a live handoff, then `herdr server restart`, then `herdr server stop`.
#
# Everything lives under one scratch folder: herdr's config and sockets, and
# the herdr-core session `herdrdev-<session>` it starts there (core follows
# XDG_RUNTIME_DIR and XDG_STATE_HOME). A user's herdr and core are never
# reached.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERDR_BIN="${HERDR_BIN:-$ROOT/target/debug/herdr}"
BASE="$(mktemp -d /tmp/hcl.XXXXXX)"
CONFIG_HOME="$BASE/c"
RUNTIME_DIR="$BASE/r"
STATE_DIR="$BASE/s"
SESSION="v10"
CORE_SESSION="herdrdev-$SESSION"
SOCKET="$CONFIG_HOME/herdr-dev/sessions/$SESSION/herdr.sock"

case "$BASE" in
  /tmp/hcl.*) ;;
  *) echo "refusing scratch folder $BASE" >&2; exit 1 ;;
esac

run_herdr() {
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
    -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    SHELL=/bin/sh \
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

# D10: a pane that was reattached is never reported as an orphan
# (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/284).
no_orphan_report() {
  if grep -h "has no pane" "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.log 2>/dev/null; then
    fail "$1 reported a reattached core terminal as having no pane"
  fi
}

fail() {
  echo "FAIL: $*" >&2
  echo "--- server log tail ---" >&2
  tail -n 40 "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.log 2>/dev/null >&2 || true
  if [[ -s "$BASE/restart.out" ]]; then
    echo "--- herdr server restart output ---" >&2
    tail -n 20 "$BASE/restart.out" >&2
  fi
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
  api "$(python3 -I -c 'import json,sys; print(json.dumps({"id":"v10","method":"pane.send_input","params":{"pane_id":sys.argv[1],"text":sys.argv[2],"keys":["Enter"]}}))' "$1" "$2")" >/dev/null
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
gone() { ! kill -0 "$1" 2>/dev/null; }
socket_up() { [[ -S "$SOCKET" ]] && api '{"id":"v10","method":"ping","params":{}}' >/dev/null 2>&1; }

# One line per live core terminal of this session: address, pid, pane label.
core_terminals() {
  local address
  for address in $(hc_scratch ls --json --label "herdr.session=$CORE_SESSION" 2>/dev/null \
    | python3 -I -c 'import json,sys; [print(t["address"]) for t in json.load(sys.stdin)["terminals"] if t["state"] == "live"]'); do
    hc_scratch get "$address" --json \
      | python3 -I -c 'import json,sys; t=json.load(sys.stdin); print(t["address"], t.get("pid") or 0, (t.get("labels") or {}).get("herdr.pane_id","-"))'
  done
}

set_runtime() {
  printf 'onboarding = false\n\n[experimental]\nruntime = "%s"\ncore_status_mirror = true\ncore_spaces_doc = %s\n' \
    "$1" "${2:-true}" > "$CONFIG_HOME/herdr-dev/config.toml"
}

# Core's `herdr-app` document as `<spaces> | <leaves>`: each space id, with
# `+` when herdr-max marked it, then each terminal block as `pane=address`
# (`null` when there is no document). doc.get has no CLI verb: plain-mode
# JSON-RPC through `hc connect`.
doc_summary() {
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"doc.get","params":{"name":"herdr-app"}}' \
    | hc_scratch connect \
    | python3 -I -c '
import json, sys
value = json.load(sys.stdin)["result"]["value"]
def blocks(node):
    if node.get("type") == "block":
        yield "%s=%s" % (node["id"], node["content"]["terminal"])
    for child in node.get("children", []):
        yield from blocks(child)
if value is None:
    print("null")
else:
    spaces = value["spaces"]
    print(" ".join(("+" if "herdr_max" in s else "") + s["id"] for s in spaces), "|",
          " ".join(leaf for s in spaces for tab in s["tabs"] for leaf in blocks(tab["root"])))'
}
# The id of herdr-max's marked space (the one workspace).
our_space() { doc_summary | tr ' ' '\n' | sed -n 's/^+//p' | head -n 1; }
# Another client's write at core's current version: its own space `s9` in
# front, herdr-max's spaces dropped, as a phone that lost them would write.
FOREIGN='{"id":"s9","name":"phone","folder":"/p","color":"teal","tabs":[]}'
doc_clobber() {
  local version
  version="$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"doc.get","params":{"name":"herdr-app"}}' \
    | hc_scratch connect | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["result"]["version"])')"
  printf '{"jsonrpc":"2.0","id":2,"method":"doc.set","params":{"name":"herdr-app","value":{"spaces":[%s]},"version":%s}}\n' \
    "$FOREIGN" "$version" | hc_scratch connect | grep -q '"result"' || fail "the foreign doc.set was refused"
}
published() { [[ "$(doc_summary)" == "+$(our_space) | $B=$ADDR" ]]; }
merged() { [[ "$(doc_summary)" == "s9 +$(our_space) | $B=$ADDR" ]]; }
withdrawn() { [[ "$(doc_summary)" == "s9 | " ]]; }

echo "herdr: $HERDR_BIN"
echo "scratch: $BASE"
mkdir -p "$CONFIG_HOME/herdr-dev" "$RUNTIME_DIR" "$STATE_DIR"

# 1. A PTY pane, then a core pane, in one server.
set_runtime pty
run_herdr server >/dev/null 2>&1 &
SERVER_PID=$!
wait_until "server socket" socket_up
A="$(api "{\"id\":\"v10\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])')"
send_line "$A" "echo \$\$ > $BASE/a.pid"
wait_until "pane A pid" file_has_line "$BASE/a.pid"
A_PID="$(cat "$BASE/a.pid")"

set_runtime core
run_herdr server reload-config >/dev/null
B="$(api "{\"id\":\"v10\",\"method\":\"pane.split\",\"params\":{\"target_pane_id\":\"$A\",\"direction\":\"right\"}}" \
  | python3 -I -c 'import json,sys; r=json.load(sys.stdin); print((r.get("pane") or r)["pane_id"])')"
send_line "$B" "echo \$\$ > $BASE/b.pid; echo V10-B-SCREEN"
wait_until "pane B pid" file_has_line "$BASE/b.pid"
B_PID="$(cat "$BASE/b.pid")"

read -r ADDR CORE_PID LABEL <<<"$(core_terminals)"
[[ "$(core_terminals | wc -l | tr -d ' ')" == 1 ]] || fail "expected one core terminal: $(core_terminals)"
[[ "$CORE_PID" == "$B_PID" ]] || fail "core terminal pid $CORE_PID is not pane B's shell $B_PID"
[[ "$LABEL" == "$B" ]] || fail "core terminal labelled $LABEL, pane B is $B"
echo "ok: pane A ($A, pid $A_PID) on the PTY, pane B ($B, pid $B_PID) in core as $ADDR"

# V15 (D2): the core our server started is not ours to end: not our child, not
# in our session, and started without a pane's or an agent's variables.
core_pid() { hc_scratch server status --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["pid"])'; }
sid() { python3 -I -c 'import os,sys; print(os.getsid(int(sys.argv[1])))' "$1"; }
CORE="$(core_pid)"
[[ "$(ps -o ppid= -p "$CORE" | tr -d ' ')" != "$SERVER_PID" ]] || fail "core $CORE is our server's child"
[[ "$(sid "$CORE")" != "$(sid "$SERVER_PID")" ]] || fail "core $CORE is in our server's session"
# What core hands its programs is what counts (it keeps its starter's
# environment, phase 0): run one from a clean caller environment and read it.
env -i HOME="$HOME" PATH="/usr/bin:/bin:/opt/homebrew/bin:$HOME/.local/bin" \
  XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" HC_SESSION="$CORE_SESSION" \
  hc run --bg "env > $BASE/core.env" >/dev/null
wait_until "the core's environment" file_has_line "$BASE/core.env"
grep -q '^PATH=' "$BASE/core.env" || fail "could not read the core's environment"
if grep -Eq '^(HERDR_PANE_ID|HERDR_TAB_ID|HERDR_WORKSPACE_ID|CLAUDE_CODE_[A-Z_]*|ANTHROPIC_[A-Z_]*|CLAUDECODE)=' "$BASE/core.env"; then
  fail "core hands its programs a pane's or an agent's variables: $(grep -E '^(HERDR_|CLAUDE|ANTHROPIC)' "$BASE/core.env" | cut -d= -f1 | tr '\n' ' ')"
fi
echo "ok: core $CORE runs outside our server's session, without pane or agent variables"

# V11: a hook inside a core pane still reports through our API, to its pane.
pane_get() {
  api "{\"id\":\"v10\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"$1\"}}"
}
# The sleep keeps B's shell out of the foreground: core clears a status when
# the root shell comes back (herdr-core `foreground.rs`).
send_line "$B" "\"$HERDR_BIN\" pane report-agent \"\$HERDR_PANE_ID\" --source v11:test --agent pi --state working > $BASE/report.out 2>&1; echo \$? > $BASE/report.rc; sleep 3"
wait_until "the hook report from inside B" file_has_line "$BASE/report.rc"
[[ "$(cat "$BASE/report.rc")" == 0 ]] || fail "report-agent failed in B: $(cat "$BASE/report.out")"
B_AGENT="$(pane_get "$B" | python3 -I -c 'import json,sys; r=json.load(sys.stdin); p=r.get("pane", r); print(p.get("agent"), p.get("agent_status"))')"
[[ "$B_AGENT" == "pi working" ]] || fail "B's hook report did not land on B: $B_AGENT ($(pane_get "$B"))"
echo "ok: a hook in core pane B reported to B through our API"
core_status() {
  hc_scratch get "$ADDR" --json \
    | python3 -I -c 'import json,sys; s=json.load(sys.stdin).get("status") or {}; print(s.get("state","-"), s.get("app","-"))'
}
mirrored() { [[ "$(core_status)" == "working pi" ]]; }
wait_until "B's status mirrored into core" mirrored
echo "ok: core_status_mirror put B's status in core: $(core_status)"
wait_until "the layout in core's herdr-app document" published
echo "ok: core_spaces_doc published B as $(doc_summary); PTY pane A is left out"

# 2. Live handoff: both programs keep running; B stays the same core terminal.
run_herdr server live-handoff --import-exe "$HERDR_BIN" >/dev/null \
  || fail "live handoff with a core pane did not complete (R1)"
wait_until "server after handoff" socket_up
alive "$A_PID" || fail "PTY pane's program ended in the handoff"
alive "$B_PID" || fail "core pane's program ended in the handoff"
[[ "$(core_terminals)" == "$ADDR $CORE_PID $B" ]] || fail "core terminals changed: $(core_terminals)"
send_line "$B" "echo after-handoff > $BASE/b.handoff"
wait_until "input to B after the handoff" file_has_line "$BASE/b.handoff"
[[ "$(core_pid)" == "$CORE" ]] || fail "core changed in the handoff"
no_orphan_report "the live handoff"
echo "ok: live handoff kept both programs and the core; B reattached to $ADDR and takes input"

doc_clobber
[[ "$(doc_summary)" == "s9 | " ]] || fail "the foreign write did not land: $(doc_summary)"

# 3. herdr server restart: the PTY pane starts fresh on the PTY (D1), the core
#    pane is kept and reattached.
run_herdr server restart --yes >"$BASE/restart.out" 2>&1 &
wait_until "PTY program ended by the restart" gone "$A_PID"
wait_until "server after restart" socket_up
alive "$B_PID" || fail "core pane's program ended in the restart"
[[ "$(core_terminals)" == "$ADDR $CORE_PID $B" ]] \
  || fail "expected only B's kept terminal (A restores on the PTY, D1): $(core_terminals)"
send_line "$B" "echo after-restart > $BASE/b.restart"
wait_until "input to B after the restart" file_has_line "$BASE/b.restart"
[[ "$(core_pid)" == "$CORE" ]] || fail "core changed in the restart"
no_orphan_report "the restart"
echo "ok: server restart kept the core and B's program in $ADDR, and reattached it"
wait_until "the restarted server's layout merged beside the foreign space" merged
set_runtime core false
run_herdr server reload-config >/dev/null
wait_until "only the foreign space left after the reload" withdrawn
echo "ok: core_spaces_doc: the restarted server kept the foreign space; reload withdrew only ours"

# 4. herdr server stop ends the core pane's program.
run_herdr server stop >/dev/null
wait_until "core pane's program ended by the stop" gone "$B_PID"
if core_terminals | grep -q "^$ADDR "; then
  fail "$ADDR still live after server stop"
fi
# B was the pane whose spawn started the core; ending it (the D8 close path)
# and stopping our server leave the core running.
alive "$CORE" || fail "core ended with our server"
echo "ok: server stop ended B's program and left the core $CORE running"
echo "PASS: core live test (V10 server half, V11, V15, core-spaces-doc)"
