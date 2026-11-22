#!/usr/bin/env bash
# Server-level herdr-core checks (openspec/changes/core-backend): a throwaway
# herdr with one PTY pane and one herdr-core pane. V15: the core it starts is
# not tied to it; V11: a hook in the core pane reports through our API, and
# with `core_status_mirror` on, the status reaches core's record
# (openspec/changes/core-status-mirror); with `core_spaces_doc` on, the layout
# merges into core's `herdr-app` document beside another client's space
# (openspec/changes/core-spaces-doc); V10: a live handoff, then `herdr server
# restart`, then `herdr server stop`. Core-phase3: V14, a holder loss suspends
# the core panes and they resume; V29, with the core server killed, a restart
# and a live handoff start nothing and the panes wait for core, which herdr
# starts again; V15, a killed core server is started again after 3 s and
# every program keeps its pid; V37, `hc server stop` suspends the panes and
# herdr waits for core; V16, a holder that keeps dying makes herdr give up on
# core until a config reload; V19, a pane in a second core session is kept
# through a handoff and a restart, and a stop ends both sessions' panes; V33,
# a pane named by `core_pty_panes` stays on the PTY when respawned, resumed
# and restored (macOS: it uses pane jobs), and the idle move refuses it; V20,
# the idle move takes idle panes onto core, an agent behind atuin too, and
# refuses busy ones.
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
# A second scratch core session, which herdr reaches but never starts (V19).
OTHER="$CORE_SESSION-b"
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
    SHELL=/bin/sh PATH="$BASE/bin:$PATH" \
    HERDR_M4_SENTINEL_TOKEN=leak SSH_AUTH_SOCK="$BASE/sentinel-agent.sock" \
    "$HERDR_BIN" --session "$SESSION" "$@"
}

hc_scratch() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HC_SESSION="$CORE_SESSION" hc "$@"
}

hc_other() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HC_SESSION="$OTHER" hc "$@"
}

cleanup() {
  set +e
  run_herdr server stop >/dev/null 2>&1
  hc_scratch server stop >/dev/null 2>&1
  # Only a running one: any hc call would start the session.
  [[ -S "$RUNTIME_DIR/herdr-core/$OTHER.sock" ]] && hc_other server stop >/dev/null 2>&1
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
  printf 'onboarding = false\n\n[experimental]\nruntime = "%s"\ncore_status_mirror = true\ncore_label_sync = true\ncore_spaces_doc = %s\n' \
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
mkdir -p "$CONFIG_HOME/herdr-dev" "$RUNTIME_DIR" "$STATE_DIR" "$BASE/bin"
# The agent stand-in for V14: what it was started with, then it stays.
cat > "$BASE/bin/p3stub" <<STUB
#!/bin/sh
echo "args=\$*" > "$BASE/stub.args"
exec sleep 600
STUB
chmod +x "$BASE/bin/p3stub"

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
# herdr's environment has two sentinels (security review M4); neither may
# reach the core, after this first start or after the recovery start (3d).
core_env_is_clean() {
  local out="$BASE/core.$1.env"
  env -i HOME="$HOME" PATH="/usr/bin:/bin:/opt/homebrew/bin:$HOME/.local/bin" \
    XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" HC_SESSION="$CORE_SESSION" \
    hc run --bg "env > $out" >/dev/null
  wait_until "the core's environment ($1)" file_has_line "$out"
  grep -q '^PATH=' "$out" || fail "could not read the core's environment ($1)"
  if grep -Eq '^(HERDR_PANE_ID|HERDR_TAB_ID|HERDR_WORKSPACE_ID|HERDR_M4_SENTINEL_TOKEN|SSH_AUTH_SOCK|CLAUDE_CODE_[A-Z_]*|ANTHROPIC_[A-Z_]*|CLAUDECODE)=' "$out"; then
    fail "core ($1) hands its programs herdr's variables: $(grep -E '^(HERDR_|SSH_AUTH_SOCK|CLAUDE|ANTHROPIC)' "$out" | cut -d= -f1 | tr '\n' ' ')"
  fi
}
core_env_is_clean first
echo "ok: core $CORE runs outside our server's session, without pane, agent or sentinel variables"

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

# core-phase4 verifier 10 and 15: with core_label_sync on, a core pane starts
# seen up to turn 0, and `herdr pane mark-seen` raises herdr.seen to the
# pane's core turn. B finishes while another workspace is focused, so it is
# unread (no client is attached, so nothing is on screen).
core_label() {
  hc_scratch get "$ADDR" --json \
    | python3 -I -c 'import json,sys; t=json.load(sys.stdin); print(t.get("turn",0), (t.get("labels") or {}).get("herdr.seen","-"))'
}
[[ "$(core_label)" == "0 0" ]] || fail "B did not start with herdr.seen=0: $(core_label)"
W2="$(api "{\"id\":\"v10\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["workspace"]["workspace_id"])')"
send_line "$B" "\"$HERDR_BIN\" pane report-agent \"\$HERDR_PANE_ID\" --source v11:test --agent pi --state idle; sleep 3"
b_done() { [[ "$(pane_get "$B" | python3 -I -c 'import json,sys; r=json.load(sys.stdin); print(r.get("pane", r).get("agent_status"))')" == done ]]; }
wait_until "B done while unread" b_done
turned() { [[ "$(core_label | cut -d' ' -f1)" -ge 1 ]]; }
wait_until "B's finished turn in core" turned
run_herdr pane mark-seen >/dev/null || fail "herdr pane mark-seen failed"
raised() { read -r turn seen <<<"$(core_label)"; [[ "$seen" == "$turn" ]]; }
wait_until "herdr.seen raised to B's turn" raised
echo "ok: core_label_sync started B at herdr.seen=0 and mark-seen raised it to its turn: $(core_label)"
api "{\"id\":\"v10\",\"method\":\"workspace.close\",\"params\":{\"workspace_id\":\"$W2\",\"force\":true}}" >/dev/null
only_b() { [[ "$(core_terminals | wc -l | tr -d ' ')" == 1 ]]; }
wait_until "the second workspace's core pane closed" only_b

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

# 3b. V14 (core-phase3 D1, D2): quitting the holder ends every core pane's
#     program with holder_lost. The panes stay, suspended, in their tab; once
#     the programs are gone they start again in place, the agent by its plan.
pane_field() {
  pane_get "$1" | python3 -I -c 'import json,sys; r=json.load(sys.stdin); p=r.get("pane", r); print(p.get(sys.argv[1]) or False)' "$2"
}
is_suspended() { [[ "$(pane_field "$1" suspended)" == True ]]; }
is_running() { [[ "$(pane_field "$1" suspended)" == False && "$(pane_field "$1" core_waiting)" == False ]]; }
is_waiting() { [[ "$(pane_field "$1" core_waiting)" == True ]]; }
wait_long() {
  local what="$1"
  shift
  for _ in $(seq 1 1200); do
    if "$@"; then return 0; fi
    sleep 0.05
  done
  fail "timed out waiting for: $what"
}
split_down() {
  api "{\"id\":\"v14\",\"method\":\"pane.split\",\"params\":{\"target_pane_id\":\"$1\",\"direction\":\"down\"}}" \
    | python3 -I -c 'import json,sys; r=json.load(sys.stdin); print((r.get("pane") or r)["pane_id"])'
}
pid_of() { core_terminals | awk -v pane="$1" '$3 == pane { print $2 }'; }
address_of() { core_terminals | awk -v pane="$1" '$3 == pane { print $1 }'; }
send_line "$A" "echo \$\$ > $BASE/a2.pid"
wait_until "pane A pid after the restart" file_has_line "$BASE/a2.pid"
A_PID="$(cat "$BASE/a2.pid")"
C="$(split_down "$B")"
send_line "$C" "echo \$\$ > $BASE/c.pid"
wait_until "pane C pid" file_has_line "$BASE/c.pid"
D="$(split_down "$C")"
send_line "$D" "\"$HERDR_BIN\" pane report-agent \"\$HERDR_PANE_ID\" --source v14:test --agent claude --state idle --agent-session-id sess-p3 -- p3stub --resume sess-p3 > $BASE/d.report 2>&1; echo \$\$ > $BASE/d.pid; exec sleep 600"
wait_until "pane D reported" file_has_line "$BASE/d.pid"
[[ "$(core_terminals | wc -l | tr -d ' ')" == 3 ]] || fail "expected three core panes: $(core_terminals)"
OLD_PIDS="$(core_terminals | awk '{print $2}' | sort | tr '\n' ' ')"
HOLDER="$(hc_scratch server status --json | python3 -I -c 'import json,sys; h=json.load(sys.stdin)["holders"]; print(h[0]["pid"] if h else 0)')"
[[ "$HOLDER" != 0 ]] || fail "no holder to quit"
kill -TERM "$HOLDER"
for pane in "$B" "$C" "$D"; do
  wait_until "pane $pane suspended by the holder loss" is_suspended "$pane"
done
alive "$A_PID" || fail "the PTY pane ended with the holder"
echo "ok: the holder loss suspended B, C and D in place; A runs on"
# Three ends within 2 s are a burst: the panes settle 30 s before they resume.
for pane in "$B" "$C" "$D"; do
  wait_long "pane $pane resumed" is_running "$pane"
done
wait_until "the agent stand-in resumed" file_has_line "$BASE/stub.args"
[[ "$(cat "$BASE/stub.args")" == "args=--resume sess-p3" ]] || fail "the stand-in ran with $(cat "$BASE/stub.args")"
[[ "$(core_terminals | wc -l | tr -d ' ')" == 3 ]] || fail "expected three core panes after the resume: $(core_terminals)"
NEW_PIDS="$(core_terminals | awk '{print $2}' | sort | tr '\n' ' ')"
[[ "$NEW_PIDS" != "$OLD_PIDS" ]] || fail "the panes did not start new programs"
NEW_HOLDER="$(hc_scratch server status --json | python3 -I -c 'import json,sys; h=json.load(sys.stdin)["holders"]; print(h[0]["pid"] if h else 0)')"
[[ "$NEW_HOLDER" != 0 && "$NEW_HOLDER" != "$HOLDER" ]] || fail "the panes are not on a new holder ($NEW_HOLDER)"
echo "ok: B, C and D started again on holder $NEW_HOLDER; the agent resumed with --resume sess-p3"

# 3c. V29a (core-phase3 D1a, D1b): with the core server killed (its socket
#     stays and refuses; the holder and the programs run on), a restart and a
#     live handoff start nothing for the core panes: they wait for core, and
#     attach to the same programs once herdr has started the core again (D3,
#     3 s down). No hc call here before that: any hc call would start a core.
core_terminals > "$BASE/before"
before_of() { awk -v pane="$1" '$3 == pane { print $1, $2 }' "$BASE/before"; }
before_pid() { awk -v pane="$1" '$3 == pane { print $2 }' "$BASE/before"; }
CORE="$(core_pid)"
kill -KILL "$CORE"
wait_until "core server gone" gone "$CORE"
run_herdr server restart --yes >"$BASE/restart2.out" 2>&1 &
wait_until "PTY program ended by the second restart" gone "$A_PID"
wait_until "server after the second restart" socket_up
for pane in "$B" "$C" "$D"; do
  wait_until "pane $pane waiting for core after the restart" is_waiting "$pane"
  alive "$(before_pid "$pane")" || fail "pane $pane's program ended in the restart"
done
run_herdr server live-handoff --import-exe "$HERDR_BIN" >/dev/null \
  || fail "live handoff with panes waiting for core did not complete"
wait_until "server after the handoff" socket_up
for pane in "$B" "$C" "$D"; do
  # Waiting, or already attached to its own program if herdr started the
  # core in the meantime: never a fresh program.
  is_waiting "$pane" || is_running "$pane" || fail "pane $pane neither waits nor runs after the handoff"
  alive "$(before_pid "$pane")" || fail "pane $pane's program ended in the handoff"
done
echo "ok: with the core server down, a restart and a handoff started nothing for B, C and D"
for pane in "$B" "$C" "$D"; do
  wait_long "pane $pane attached once core runs" is_running "$pane"
  [[ "$(address_of "$pane") $(pid_of "$pane")" == "$(before_of "$pane")" ]] \
    || fail "pane $pane is on $(address_of "$pane") $(pid_of "$pane"), not $(before_of "$pane")"
done
[[ "$(core_terminals | wc -l | tr -d ' ')" == 3 ]] || fail "a program was started: $(core_terminals)"
send_line "$B" "echo after-wait > $BASE/b.wait"
wait_until "input to B after it attached" file_has_line "$BASE/b.wait"
echo "ok: B, C and D attached to the same programs once herdr started the core; nothing was started twice"

# 3d. V15 (core-phase3 D3): the core server killed while herdr runs. Its
#     socket stays and refuses; after 3 s herdr starts the core, which finds
#     the live holder: every program keeps its pid and nothing is suspended.
#     No hc call until the core is back: any hc call would start one.
CORE_SOCKET="$RUNTIME_DIR/herdr-core/$CORE_SESSION.sock"
core_socket_state() {
  python3 -I - "$CORE_SOCKET" <<'PY2'
import os, socket, sys
if not os.path.exists(sys.argv[1]):
    print("missing")
    raise SystemExit
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
try:
    client.connect(sys.argv[1])
    print("up")
except ConnectionRefusedError:
    print("refused")
PY2
}
core_up() { [[ "$(core_socket_state)" == up ]]; }
core_terminals > "$BASE/before15"
CORE="$(core_pid)"
kill -KILL "$CORE"
wait_until "core server gone" gone "$CORE"
KILLED_AT="$(python3 -I -c 'import time; print(time.monotonic())')"
[[ "$(core_socket_state)" == refused ]] || fail "the killed core's socket is $(core_socket_state), not refused"
wait_long "herdr started the core again" core_up
GAP="$(python3 -I -c 'import sys,time; print(f"{time.monotonic()-float(sys.argv[1]):.1f}")' "$KILLED_AT")"
python3 -I -c 'import sys; sys.exit(0 if float(sys.argv[1]) >= 3.0 else 1)' "$GAP" \
  || fail "herdr started the core after only $GAP s"
for pane in "$B" "$C" "$D"; do
  wait_long "pane $pane reattached after the core came back" is_running "$pane"
done
[[ "$(core_terminals | sort)" == "$(sort "$BASE/before15")" ]] \
  || fail "programs changed across the core server loss: $(core_terminals) vs $(cat "$BASE/before15")"
[[ "$(core_pid)" != "$CORE" ]] || fail "the core pid did not change"
send_line "$B" "echo after-v15 > $BASE/b.v15"
wait_until "input to B after the core came back" file_has_line "$BASE/b.v15"
core_env_is_clean recovery
echo "ok: herdr started the killed core again after $GAP s; B, C and D kept their programs; its environment is clean"

# 3e. V37 (core-phase3 D3b): `hc server stop` is a person's stop. The panes
#     are suspended, `core_stopped`, and herdr does not start the core; once
#     someone starts it, each pane resumes after the dead-program check.
OLD_PIDS="$(core_terminals | awk '{print $2}' | sort | tr '\n' ' ')"
hc_scratch server stop >/dev/null
for pane in "$B" "$C" "$D"; do
  wait_until "pane $pane suspended by the stop" is_suspended "$pane"
  [[ "$(pane_field "$pane" suspend_cause)" == core_stopped ]] \
    || fail "pane $pane suspended as $(pane_field "$pane" suspend_cause)"
done
for _ in $(seq 1 60); do
  [[ "$(core_socket_state)" == missing ]] || fail "herdr started the core a person stopped"
  for pane in "$B" "$C" "$D"; do
    is_suspended "$pane" || fail "pane $pane resumed while the core was stopped"
  done
  sleep 1
done
echo "ok: after hc server stop, B, C and D stayed suspended and no core was started for 60 s"
hc_scratch server start >/dev/null
for pane in "$B" "$C" "$D"; do
  wait_long "pane $pane resumed once the core runs" is_running "$pane"
done
NEW_PIDS="$(core_terminals | awk '{print $2}' | sort | tr '\n' ' ')"
[[ "$(core_terminals | wc -l | tr -d ' ')" == 3 ]] || fail "expected three core panes: $(core_terminals)"
[[ "$NEW_PIDS" != "$OLD_PIDS" ]] || fail "the panes did not start new programs"
echo "ok: once the core was started by hand, B, C and D resumed on it"

#     V37, second run: stopped again, then a herdr restart, whose first
#     connection may start the core; the panes come back.
hc_scratch server stop >/dev/null
for pane in "$B" "$C" "$D"; do
  wait_until "pane $pane suspended by the second stop" is_suspended "$pane"
done
run_herdr server restart --yes >"$BASE/restart3.out" 2>&1 &
wait_until "server after the third restart" socket_up
for pane in "$B" "$C" "$D"; do
  wait_long "pane $pane running after the restart" is_running "$pane"
done
# Restored panes start on the PTY until PR F (core-phase3 D8).
echo "ok: after a second stop, a herdr restart brought B, C and D back"

# 3f. V16 (core-phase3 D3a): a holder that keeps dying. Each loss suspends
#     the core panes and they resume on a new holder; the third burst in ten
#     minutes makes herdr give up: the panes resume on the PTY and `herdr
#     server status` says `core: gave_up` until a config reload.
E="$(split_down "$A")"
send_line "$E" "echo \$\$ > $BASE/e.pid"
wait_until "pane E pid" file_has_line "$BASE/e.pid"
F="$(split_down "$E")"
send_line "$F" "echo \$\$ > $BASE/f.pid"
wait_until "pane F pid" file_has_line "$BASE/f.pid"
on_core() { [[ -n "$(pid_of "$1")" ]]; }
on_core "$E" || fail "pane E is not on core: $(core_terminals)"
on_core "$F" || fail "pane F is not on core: $(core_terminals)"
server_core_field() {
  run_herdr status server --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("core") or "-")'
}
[[ "$(server_core_field)" == - ]] || fail "a new server reports core $(server_core_field)"
for round in 1 2 3; do
  HOLDER="$(hc_scratch server status --json | python3 -I -c 'import json,sys; h=json.load(sys.stdin)["holders"]; print(h[0]["pid"] if h else 0)')"
  [[ "$HOLDER" != 0 ]] || fail "round $round: no holder to quit"
  kill -TERM "$HOLDER"
  for pane in "$E" "$F"; do
    wait_until "round $round: pane $pane suspended" is_suspended "$pane"
  done
  for pane in "$E" "$F"; do
    wait_long "round $round: pane $pane resumed" is_running "$pane"
  done
  if (( round < 3 )); then
    on_core "$E" || fail "round $round: E resumed off core"
    [[ "$(server_core_field)" == - ]] || fail "round $round: gave up too early"
  fi
done
[[ "$(server_core_field)" == gave_up ]] || fail "server status says core $(server_core_field), not gave_up"
on_core "$E" && fail "the third resume of E ran on core"
on_core "$F" && fail "the third resume of F ran on core"
grep -q "keeps ending panes" "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.log \
  || fail "no keeps-ending line in the server log"
echo "ok: the third holder loss made herdr give up; E and F resumed on the PTY; status says core: gave_up"
run_herdr server reload-config >/dev/null
[[ "$(server_core_field)" == - ]] || fail "a reload did not reset the guard: $(server_core_field)"
echo "ok: a config reload reset the guard"

G="$(split_down "$F")"
send_line "$G" "echo \$\$ > $BASE/g.pid"
wait_until "pane G pid" file_has_line "$BASE/g.pid"
on_core "$G" || fail "after the reload, a new pane G is not on core: $(core_terminals)"
ADDR="$(address_of "$G")"

# 3g. V19 (core-phase3 D6): a pane in a second core session. With our server
#     killed (core programs live on), pane G's saved link is pointed at a
#     program labelled for G in the scratch session $OTHER. The next server
#     attaches G there without starting anything, keeps it through a live
#     handoff and a restart, and names a stray terminal there with that
#     session's look-up; `herdr server stop` ends the panes in both sessions.
other_run() {
  hc_other run --bg --json --no-parent --label "herdr.session=$CORE_SESSION" \
    --label "herdr.pane_id=$1" -- /bin/sh -c 'echo $$ > "$0"; exec /bin/sh' "$2" \
    | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["address"])'
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
# Points the saved link of the pane on $1 in our session's saved files at
# $2 in $OTHER, as a link from before the pid fields; prints the count.
relink() {
  python3 -I - "$CONFIG_HOME/herdr-dev/sessions/$SESSION" "$CORE_SESSION" "$1" "$OTHER" "$2" <<'PY2'
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
saved_with_g() { grep -q "\"$ADDR\"" "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.json 2>/dev/null; }
wait_until "G's link saved" saved_with_g
G_OTHER="$(other_run "$G" "$BASE/g.other.pid")"
STRAY="$(other_run w9-stray "$BASE/stray.pid")"
wait_until "G's program in $OTHER" file_has_line "$BASE/g.other.pid"
G_OTHER_PID="$(cat "$BASE/g.other.pid")"
OTHER_CORE="$(hc_other server status --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["pid"])')"
HERDR_PID="$(server_pid)"
kill -KILL "$HERDR_PID"
wait_until "our server gone" gone "$HERDR_PID"
hc_scratch kill "$ADDR" >/dev/null
[[ "$(relink "$ADDR" "$G_OTHER")" -ge 1 ]] || fail "no saved link of G on $ADDR to point at $OTHER"
run_herdr server >/dev/null 2>&1 &
wait_until "server after the kill" socket_up
wait_long "pane G attached in $OTHER" is_running "$G"
send_line "$G" "echo \$\$ > $BASE/g.after"
wait_until "input to G in $OTHER" file_has_line "$BASE/g.after"
[[ "$(cat "$BASE/g.after")" == "$G_OTHER_PID" ]] || fail "G runs $(cat "$BASE/g.after"), not $OTHER's $G_OTHER_PID"
[[ -z "$(pid_of "$G")" ]] || fail "a program was started for G in $CORE_SESSION: $(core_terminals)"
stray_reported() {
  grep -h "has no pane" "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/*.log 2>/dev/null \
    | grep "session=$OTHER" | grep -q "address=$STRAY"
}
wait_until "the stray terminal in $OTHER reported with its session" stray_reported
echo "ok: G attached to its program in $OTHER (pid $G_OTHER_PID); the stray $STRAY there was reported as $OTHER's"
hc_other kill "$STRAY" >/dev/null

run_herdr server live-handoff --import-exe "$HERDR_BIN" >/dev/null \
  || fail "live handoff with a pane in $OTHER did not complete"
wait_until "server after the V19 handoff" socket_up
alive "$G_OTHER_PID" || fail "G's program in $OTHER ended in the handoff"
send_line "$G" "echo after-handoff > $BASE/g.handoff"
wait_until "input to G after the handoff" file_has_line "$BASE/g.handoff"
HERDR_PID="$(server_pid)"
run_herdr server restart --yes >"$BASE/restart-v19.out" 2>&1 &
wait_long "the old server gone in the V19 restart" gone "$HERDR_PID"
wait_until "server after the V19 restart" socket_up
wait_long "pane G back after the restart" is_running "$G"
alive "$G_OTHER_PID" || fail "G's program in $OTHER ended in the restart"
send_line "$G" "echo \$\$ > $BASE/g.restart"
wait_until "input to G after the restart" file_has_line "$BASE/g.restart"
[[ "$(cat "$BASE/g.restart")" == "$G_OTHER_PID" ]] || fail "after the restart G runs $(cat "$BASE/g.restart")"
[[ "$(hc_other server status --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["pid"])')" == "$OTHER_CORE" ]] \
  || fail "the core of $OTHER changed"
echo "ok: G kept its program in $OTHER through a live handoff and a restart"

H="$(split_down "$G")"
send_line "$H" "echo \$\$ > $BASE/h.pid"
wait_until "pane H pid" file_has_line "$BASE/h.pid"
on_core "$H" || fail "new pane H is not in $CORE_SESSION: $(core_terminals)"
H_PID="$(pid_of "$H")"
CORE="$(core_pid)"

# 4. herdr server stop ends the core panes' programs, in both sessions.
run_herdr server stop >/dev/null
wait_until "core pane H's program ended by the stop" gone "$H_PID"
wait_until "pane G's program in $OTHER ended by the stop" gone "$G_OTHER_PID"
[[ -z "$(core_terminals)" ]] || fail "core terminals still live after server stop: $(core_terminals)"
# Ending our server's panes (the D8 close path) and stopping our server
# leave the cores running.
alive "$CORE" || fail "core ended with our server"
alive "$OTHER_CORE" || fail "the core of $OTHER ended with our server"
echo "ok: server stop ended H's program and G's in $OTHER, and left both cores running"
hc_other server stop >/dev/null 2>&1 || true
# 5. V33 (core-phase3 D13): a pane named `test-hlp` by its agent's `--name`
#    stays on the PTY with runtime = "core" and core_pty_panes = ["*-hlp"]:
#    respawned, resumed after its pane job ended, and restored after a
#    restart. Its own herdr session, so its own core session.
hc_scratch server stop >/dev/null 2>&1 || true
SESSION="v33"
CORE_SESSION="herdrdev-$SESSION"
SOCKET="$CONFIG_HOME/herdr-dev/sessions/$SESSION/herdr.sock"
set_v33_config() {
  printf 'onboarding = false\n\n[experimental]\nruntime = "%s"\npane_jobs = true\ncore_pty_panes = [%s]\n' \
    "$1" "$2" > "$CONFIG_HOME/herdr-dev/config.toml"
}
# A compiled stand-in, so detection sees a process named claude.
cat > "$BASE/claude.c" <<STUB
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <unistd.h>
int main(int argc, char **argv) {
  /* As Claude did in fork issue 301: it outlives the hangup and a SIGTERM. */
  if (getenv("STUB_STUBBORN")) {
    signal(SIGHUP, SIG_IGN);
    signal(SIGTERM, SIG_IGN);
  }
  const char *path = getenv("STUB_ARGS");
  FILE *out = fopen(path ? path : "$BASE/h.args", "w");
  if (!out) return 1;
  /* Two children in its process group, as Claude's MCP servers. */
  if (getenv("STUB_CHILDREN"))
    for (int i = 0; i < 2; i++)
      if (fork() == 0) for (;;) pause();
  fprintf(out, "%d", (int)getpid());
  for (int i = 1; i < argc; i++) fprintf(out, " %s", argv[i]);
  fprintf(out, "\n");
  fclose(out);
  /* An empty Claude-like input box, for the idle move's draft check;
     no line feed after it, so it fits a three-row pane. */
  printf("\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\n\xe2\x9d\xaf \n\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80\xe2\x94\x80");
  fflush(stdout);
  for (;;) pause();
}
STUB
cc -o "$BASE/bin/claude" "$BASE/claude.c" || fail "could not compile the claude stand-in"
# A stand-in for `atuin pty-proxy`: it runs an interactive shell in a PTY of
# its own, so an agent started there is one PTY below the pane's.
cat > "$BASE/atuin.c" <<'STUB'
#include <sys/ioctl.h>
#include <sys/select.h>
#include <termios.h>
#include <unistd.h>
#include <util.h>
int main(void) {
  struct winsize size;
  struct termios raw;
  char buf[4096];
  int master;
  ioctl(0, TIOCGWINSZ, &size);
  pid_t child = forkpty(&master, NULL, NULL, &size);
  if (child < 0) return 1;
  if (child == 0) {
    execl("/bin/sh", "sh", "-i", (char *)0);
    _exit(127);
  }
  tcgetattr(0, &raw);
  cfmakeraw(&raw);
  tcsetattr(0, TCSANOW, &raw);
  for (;;) {
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(0, &fds);
    FD_SET(master, &fds);
    if (select(master + 1, &fds, NULL, NULL, NULL) < 0) continue;
    int ends[2][2] = {{0, master}, {master, 1}};
    for (int i = 0; i < 2; i++) {
      if (!FD_ISSET(ends[i][0], &fds)) continue;
      ssize_t n = read(ends[i][0], buf, sizeof buf);
      if (n <= 0) return 0;
      write(ends[i][1], buf, n);
    }
  }
}
STUB
cc -o "$BASE/bin/atuin" "$BASE/atuin.c" || fail "could not compile the atuin stand-in"
stub_pid() { cut -d' ' -f1 "$BASE/h.args"; }
stub_args() { cut -d' ' -f2- "$BASE/h.args"; }
stub_restarted() { file_has_line "$BASE/h.args" && [[ "$(stub_pid)" != "$1" ]] && alive "$(stub_pid)"; }
on_pty() { [[ -z "$(pid_of "$1")" ]] && is_running "$1"; }
not_a_core_program() {
  if core_terminals | awk '{print $2}' | grep -qx "$1"; then
    fail "pid $1 runs in a core terminal"
  fi
}
set_v33_config pty '"*-hlp"'
run_herdr server >/dev/null 2>&1 &
wait_until "v33 server socket" socket_up
W="$(api "{\"id\":\"v33\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])')"
H="$(split_down "$W")"
send_line "$H" "\"$HERDR_BIN\" pane report-agent \"\$HERDR_PANE_ID\" --source v33:test --agent claude --state idle --agent-session-id sess-h -- claude --resume sess-h; exec claude --name test-hlp"
wait_until "the stand-in started in H" file_has_line "$BASE/h.args"
[[ "$(stub_args)" == "--name test-hlp" ]] || fail "the stand-in ran with $(stub_args)"
session_has_launch_name() { grep -q '"test-hlp"' "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/session.json 2>/dev/null; }
wait_long "H's launch name saved" session_has_launch_name

set_v33_config core '"*-hlp"'
run_herdr server reload-config >/dev/null
X="$(split_down "$W")"
wait_until "an unnamed new pane on core" on_core "$X"

STUB_PID="$(stub_pid)"
run_herdr pane respawn "$H" --resume --force >/dev/null || fail "pane respawn --resume of H failed"
wait_long "H's agent respawned" stub_restarted "$STUB_PID"
[[ "$(stub_args)" == "--resume sess-h --name test-hlp" ]] || fail "H respawned with $(stub_args)"
on_pty "$H" || fail "H respawned on core: $(core_terminals)"
not_a_core_program "$(stub_pid)"
echo "ok: V33: H, named test-hlp by --name, respawned on the PTY while a new pane went to core"

STUB_PID="$(stub_pid)"
SHELL_PID="$(ps -o ppid= -p "$STUB_PID" | tr -d ' ')"
KEEPER="$(ps -o ppid= -p "$SHELL_PID" | tr -d ' ')"
# Stopping the keeper's launchd job, as a logout does: only this pane's job.
JOB="$(launchctl list | awk -v pid="$KEEPER" '$1 == pid { print $3 }')"
[[ "$JOB" == dev.herdr.pane.* ]] || fail "H's shell parent $KEEPER is not a pane job ($JOB)"
launchctl bootout "gui/$(id -u)/$JOB" || fail "could not stop H's pane job $JOB"
wait_until "H suspended by its pane job ending" is_suspended "$H"
wait_long "H's agent resumed" stub_restarted "$STUB_PID"
wait_long "H running after the resume" is_running "$H"
on_pty "$H" || fail "H resumed on core: $(core_terminals)"
not_a_core_program "$(stub_pid)"
echo "ok: V33: after its pane job ended, H resumed on the PTY"

# A resumed Claude reports its session again through its hook; so does the
# stand-in's test here, so the restart has a session to restore.
run_herdr pane report-agent "$H" --source v33:test --agent claude --state idle \
  --agent-session-id sess-h -- claude --resume sess-h >/dev/null
session_has_session() { grep -q '"sess-h"' "$CONFIG_HOME"/herdr-dev/sessions/"$SESSION"/session.json 2>/dev/null; }
wait_long "H's session saved" session_has_session
STUB_PID="$(stub_pid)"
run_herdr server restart --yes >"$BASE/restart4.out" 2>&1 &
wait_until "H's program ended by the restart" gone "$STUB_PID"
wait_until "v33 server after the restart" socket_up
wait_long "H's agent restored" stub_restarted "$STUB_PID"
wait_long "H running after the restore" is_running "$H"
on_pty "$H" || fail "H restored on core: $(core_terminals)"
not_a_core_program "$(stub_pid)"
on_core "$X" || fail "X is not on core after the restart: $(core_terminals)"
echo "ok: V33: after a restart, H was restored on the PTY and X reattached on core"

refusals_of() {
  run_herdr pane respawn "$@" | python3 -I -c 'import json,sys; r=json.load(sys.stdin)["result"]; print(("moved " if r["moved"] else "") + ",".join(r["refusals"]))'
}
STUB_PID="$(stub_pid)"
[[ "$(refusals_of "$H" --if-idle --check)" == *excluded* ]] || fail "H's move check said $(refusals_of "$H" --if-idle --check)"
[[ "$(refusals_of "$H" --if-idle)" == *excluded* ]] || fail "H was not refused excluded"
[[ "$(stub_pid)" == "$STUB_PID" ]] && alive "$STUB_PID" || fail "the refused move touched H's agent"
on_pty "$H" || fail "H left the PTY: $(core_terminals)"
echo "ok: V33: pane respawn --if-idle refuses H as excluded and leaves it on the PTY"

set_v33_config core ''
run_herdr server reload-config >/dev/null
STUB_PID="$(stub_pid)"
run_herdr pane respawn "$H" --resume --force >/dev/null || fail "pane respawn --resume of H failed"
wait_long "H's agent respawned without the pattern" stub_restarted "$STUB_PID"
wait_until "H on core without the pattern" on_core "$H"
echo "ok: V33: with no core_pty_panes, the same respawn put H on core"
run_herdr server stop >/dev/null
hc_scratch server stop >/dev/null 2>&1 || true

# 6. V20 (core-phase3 D9): the idle move. Panes started on the PTY, then
#    runtime = "core": an idle shell moves with its directory; a shell running
#    `sleep 30` and a pane typed into just before are refused with their
#    reasons; an idle agent stand-in moves and is resumed with its plan's
#    arguments, and so does one behind the atuin stand-in, with two children
#    (fork issue 295), and one there that ignores SIGHUP and SIGTERM (fork
#    issue 301). The windows are shortened (debug build only): input 3 s,
#    agent idle 3 s, shell quiet 6 s.
SESSION="v20"
CORE_SESSION="herdrdev-$SESSION"
SOCKET="$CONFIG_HOME/herdr-dev/sessions/$SESSION/herdr.sock"
set_v33_config pty ''
HERDR_TEST_MOVE_WINDOWS_MS=3000,3000,6000 run_herdr server >/dev/null 2>&1 &
wait_until "v20 server socket" socket_up
S1="$(api "{\"id\":\"v20\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])')"
S2="$(split_down "$S1")"
S3="$(split_down "$S2")"
AG="$(split_down "$S3")"
AW="$(split_down "$S1")"
mkdir -p "$BASE/v20 dir"
send_line "$S1" "cd \"$BASE/v20 dir\""
send_line "$S2" "sleep 30"
: > "$BASE/h.args"
send_line "$AG" "exec claude --name v20-agent"
wait_until "the stand-in started in AG" file_has_line "$BASE/h.args"
AG_PID="$(stub_pid)"
# As a Claude hook does once the agent runs: its session and resume
# command, so it has a plan.
run_herdr pane report-agent "$AG" --source v20:test --agent claude --state idle \
  --agent-session-id sess-v20 -- claude --resume sess-v20 >/dev/null
AS="$(split_down "$S2")"
send_line "$AW" "exec atuin"
send_line "$AS" "exec atuin"
wrapped() {
  run_herdr pane process-info --pane "$1" | python3 -I -c '
import json, sys
info = json.load(sys.stdin)["result"]["process_info"]
sys.exit(0 if info.get("nested_foreground_processes") else 1)'
}
wait_until "the atuin stand-in's shell in AW" wrapped "$AW"
wait_until "the atuin stand-in's shell in AS" wrapped "$AS"
send_line "$AS" "STUB_ARGS=\"$BASE/s.args\" STUB_STUBBORN=1 claude --name v20-stubborn"
wait_until "the stubborn stand-in started in AS" file_has_line "$BASE/s.args"
AS_PID="$(cut -d' ' -f1 "$BASE/s.args")"
run_herdr pane report-agent "$AS" --source v20:test --agent claude --state idle \
  --agent-session-id sess-v20s -- claude --resume sess-v20s >/dev/null
send_line "$AW" "STUB_ARGS=\"$BASE/w.args\" STUB_CHILDREN=1 claude --name v20-wrapped"
wait_until "the wrapped stand-in started in AW" file_has_line "$BASE/w.args"
AW_PID="$(cut -d' ' -f1 "$BASE/w.args")"
run_herdr pane report-agent "$AW" --source v20:test --agent claude --state idle \
  --agent-session-id sess-v20w -- claude --resume sess-v20w >/dev/null
set_v33_config core ''
run_herdr server reload-config >/dev/null
# Past every window: the setup lines above were input.
for _ in $(seq 1 140); do sleep 0.05; done
send_line "$S3" "true"
[[ "$(refusals_of "$S3" --if-idle)" == *recent_input* ]] || fail "S3 typed into was not refused recent_input"
[[ "$(refusals_of "$S2" --if-idle)" == *shell_busy* ]] || fail "S2 running sleep was not refused shell_busy"
on_pty "$S2" && on_pty "$S3" || fail "a refused pane moved: $(core_terminals)"
PLAN="--resume sess-v20 --name v20-agent"
[[ "$(refusals_of "$S1" --if-idle --check)" == "" ]] || fail "S1 check said $(refusals_of "$S1" --if-idle --check)"
on_pty "$S1" || fail "a check moved S1"
[[ "$(refusals_of "$S1" --if-idle)" == "moved " ]] || fail "the idle shell S1 did not move"
wait_until "S1 on core" on_core "$S1"
send_line "$S1" "pwd -P > \"$BASE/s1.pwd\""
wait_until "S1's directory" file_has_line "$BASE/s1.pwd"
[[ "$(cat "$BASE/s1.pwd")" == "$(cd "$BASE/v20 dir" && pwd -P)" ]] || fail "S1 moved to $(cat "$BASE/s1.pwd")"
echo "ok: V20: the idle shell moved onto core in its directory; the busy and the typed-into panes were refused"
[[ "$(refusals_of "$AG" --if-idle)" == "moved " ]] || fail "the idle agent did not move: $(refusals_of "$AG" --if-idle --check)"
gone "$AG_PID" || fail "the old agent $AG_PID still runs"
wait_long "AG's agent resumed" stub_restarted "$AG_PID"
wait_until "AG on core" on_core "$AG"
[[ "$(stub_args)" == "$PLAN" ]] || fail "AG resumed with '$(stub_args)', its plan was '$PLAN'"
echo "ok: V20: the idle agent moved onto core and resumed with its plan ($PLAN)"
AW_CHILDREN="$(pgrep -P "$AW_PID" | tr '\n' ' ')"
[[ -n "$AW_CHILDREN" ]] || fail "the wrapped stand-in has no children"
: > "$BASE/h.args"
[[ "$(refusals_of "$AW" --if-idle --check)" == "" ]] || fail "AW check said $(refusals_of "$AW" --if-idle --check)"
[[ "$(refusals_of "$AW" --if-idle)" == "moved " ]] || fail "the wrapped idle agent did not move"
for pid in $AW_PID $AW_CHILDREN; do
  wait_until "the old wrapped agent's process $pid ended" gone "$pid"
done
wait_long "AW's agent resumed" file_has_line "$BASE/h.args"
wait_until "AW on core" on_core "$AW"
[[ "$(stub_args)" == "--resume sess-v20w --name v20-wrapped" ]] || fail "AW resumed with '$(stub_args)'"
echo "ok: V20: the idle agent behind atuin moved onto core, its children ended, and it resumed with its plan"
# Fork issue 301: an agent that outlives the hangup and a SIGTERM is killed,
# its pane stays, and it resumes on core with its session.
: > "$BASE/h.args"
[[ "$(refusals_of "$AS" --if-idle --check)" == "" ]] || fail "AS check said $(refusals_of "$AS" --if-idle --check)"
[[ "$(refusals_of "$AS" --if-idle)" == "moved " ]] || fail "the stubborn idle agent did not move"
wait_until "the old stubborn agent $AS_PID ended" gone "$AS_PID"
wait_long "AS's agent resumed" file_has_line "$BASE/h.args"
wait_until "AS on core" on_core "$AS"
[[ "$(stub_args)" == "--resume sess-v20s --name v20-stubborn" ]] || fail "AS resumed with '$(stub_args)'"
echo "ok: V20: an agent that outlived the hangup and SIGTERM was killed, kept its pane, and resumed on core with its session"
run_herdr server stop >/dev/null
hc_scratch server stop >/dev/null 2>&1 || true
echo "PASS: core live test (V10 server half, V11, V15, core-spaces-doc; core-phase3 V14, V15, V16, V19, V29a, V33, V37; V20)"
