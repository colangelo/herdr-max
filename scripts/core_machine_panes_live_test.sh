# Sourced by scripts/core_machine_live_test.sh after its V26 part, with its
# scratch cores, helpers and trap (openspec/changes/core-phase5, PR C).
#
# herdr-max panes on a core machine (D5): V9 and V31's pane part, V12, V10,
# V6 (restart and live handoff while the machine is away, dormant and not),
# the server-side wake on a key (V4's server part) and V11 (a long drop,
# V11_SECS, 15 min by default).
#
# The machine here is a third scratch core on this Mac reached through a
# relay script, never ssh: core's own line runs against it with six fake
# herdr integration variables in its environment, as a far core started by
# a bare `hc` inside a herdr pane would have them (F6). Its name is unique
# to this run, because a herdr state folder for it (under the passwd home,
# where herdr looks) makes it a machine herdr gates, for the dormant cases;
# the folder is removed at the end.

[[ -n "${BASE:-}" && -n "${CORE_SESSION:-}" ]] || { echo "source this from core_machine_live_test.sh" >&2; exit 1; }

PC="t$$-pc"
PC_RUNTIME="$BASE/pr"
PC_STATE="$BASE/ps"
PC_CONFIG="$BASE/pc"
# The far core's session has the local one's name, so the far herdr of V10
# uses the same `herdr.session` value as ours: only the owner suffix tells
# our panes apart (D5).
PC_SESSION="$CORE_SESSION"
PC_SOCK="$PC_RUNTIME/herdr-core/$PC_SESSION.sock"
LOCAL_CORE_SOCK="$RUNTIME_DIR/herdr-core/$CORE_SESSION.sock"
PC_ROOT="$(python3 -I -c 'import os, pwd; print(pwd.getpwuid(os.getuid()).pw_dir)')/.local/state/herdr-dev/core-machines"
PC_DIR="$PC_ROOT/$PC"
FAR_HERDR_SOCKET="$PC_CONFIG/herdr-dev/sessions/$SESSION/herdr.sock"
V11_SECS="${V11_SECS:-900}"

hc_pc() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$PC_RUNTIME" XDG_STATE_HOME="$PC_STATE" \
    HC_SESSION="$PC_SESSION" hc "$@"
}

run_far_herdr() {
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
    -u CLAUDECODE \
    XDG_CONFIG_HOME="$PC_CONFIG" XDG_RUNTIME_DIR="$PC_RUNTIME" XDG_STATE_HOME="$PC_STATE" \
    SHELL=/bin/sh HERDR_NO_MACHINE_LEASES=1 "$HERDR_BIN" --session "$SESSION" "$@"
}

link_cleanup() {
  set +e
  [[ -S "$FAR_HERDR_SOCKET" ]] && run_far_herdr server stop >/dev/null 2>&1
  [[ -S "$LOCAL_CORE_SOCK" ]] && hc_scratch machine rm "$PC" >/dev/null 2>&1
  [[ -S "$PC_SOCK" ]] && hc_pc server stop >/dev/null 2>&1
  case "$PC" in t[0-9]*-pc) rm -rf "${PC_ROOT:?}/$PC" ;; esac
}

wait_secs() {
  local what="$1" secs="$2"
  shift 2
  local until=$((SECONDS + secs))
  while ((SECONDS < until)); do
    if "$@"; then return 0; fi
    sleep 0.2
  done
  fail "timed out after ${secs}s waiting for: $what"
}

pane_json() {
  api "{\"id\":\"pc\",\"method\":\"pane.get\",\"params\":{\"pane_id\":\"$1\"}}" \
    | python3 -I -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["pane"]))'
}
pane_field() { pane_json "$1" | python3 -I -c 'import json,sys; v = json.load(sys.stdin).get(sys.argv[1]); print("" if v is None else v)' "$2"; }
pc_state() {
  hc_scratch machine ls --json 2>/dev/null | python3 -I -c 'import json,sys
d = json.load(sys.stdin)
rows = d["machines"] if isinstance(d, dict) else d
print(next((m["state"] for m in rows if m["name"] == sys.argv[1]), "absent"))' "$PC"
}
pc_is() { [[ "$(pc_state)" == "$1" ]]; }
# One call on the local core's socket; prints its result (`null` on an error).
core_rpc() {
  python3 -I -c 'import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
f = s.makefile("rb")
s.sendall(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"hello\",\"params\":{}}\n"); f.readline()
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": sys.argv[2], "params": json.loads(sys.argv[3])}) + "\n").encode())
print(json.dumps(json.loads(f.readline()).get("result")))' "$LOCAL_CORE_SOCK" "$1" "$2"
}
far_terminal() { core_rpc terminal.get "{\"terminal\":\"$1\"}"; }
far_live() { far_terminal "$1" | python3 -I -c 'import json,sys; t = json.load(sys.stdin); sys.exit(0 if t and t["state"] == "live" else 1)'; }
# Ends the link: the relay records its pid, which core's dial execs into.
drop_link() {
  [[ -s "$BASE/pc.via.pid" ]] && kill "$(cat "$BASE/pc.via.pid")" 2>/dev/null
  true
}
waiting_cause() { pane_field "$1" core_waiting_cause; }
cause_is() { [[ "$(waiting_cause "$1")" == "$2" ]]; }
not_waiting() { [[ -z "$(waiting_cause "$1")" && "$(pane_field "$1" core_waiting)" != "True" ]]; }
write_lease() {
  local now
  now="$(python3 -I -c 'import time; print(f"{time.time():.3f}")')"
  python3 -I -c 'import os,sys
d, holder, exp, act = sys.argv[1:]
os.makedirs(os.path.join(d, "leases"), mode=0o700, exist_ok=True)
tmp = os.path.join(d, "leases", holder + ".new")
open(tmp, "w").write(f"{exp} {act}")
os.replace(tmp, os.path.join(d, "leases", holder))' "$PC_DIR" "cli-$$" "$(python3 -I -c "print($now + $1)")" "$now"
}

echo "--- PR C: panes on core machine $PC"
mkdir -p "$PC_RUNTIME" "$PC_STATE" "$PC_CONFIG/herdr-dev"
printf 'onboarding = false\n\n[experimental]\nruntime = "core"\n' > "$PC_CONFIG/herdr-dev/config.toml"
# The relay to the machine. The far core it starts inherits this
# environment, so it carries the six variables (F6).
cat > "$BASE/via_pc.sh" <<VIA
#!/bin/sh
for line; do :; done
[ -e "$BASE/pc.down" ] && exit 1
echo \$\$ > "$BASE/pc.via.pid"
exec env -u HC_SESSION XDG_RUNTIME_DIR="$PC_RUNTIME" XDG_STATE_HOME="$PC_STATE" \
  XDG_CONFIG_HOME="$PC_CONFIG" \
  HERDR_ENV=1 HERDR_PANE_ID=w9:p9 HERDR_TAB_ID=w9:t9 HERDR_WORKSPACE_ID=w9 \
  HERDR_SOCKET_PATH="$BASE/nowhere.sock" HERDR_CLIENT_SOCKET_PATH="$BASE/nowhere-client.sock" \
  sh -c "\$line"
VIA
chmod +x "$BASE/via_pc.sh"
hc_scratch machine add --json --name "$PC" --session "$PC_SESSION" --via "$BASE/via_pc.sh" >/dev/null \
  || fail "machine add $PC failed"
wait_secs "$PC connected" 15 pc_is connected
CORE_ID="$(python3 -I - "$LOCAL_CORE_SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall(b'{"jsonrpc":"2.0","id":0,"method":"hello","params":{}}\n')
print(json.loads(s.makefile("rb").readline())["result"]["core"]["id"])
PY
)"
[[ -n "$CORE_ID" ]] || fail "no core id"

# A local workspace to split from.
W="$(api "{\"id\":\"pc\",\"method\":\"workspace.create\",\"params\":{\"cwd\":\"$BASE\",\"focus\":true}}" \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["root_pane"]["pane_id"])')"

# --- V9: a pane on the machine: label, env wrapper, no integration
# variable, no pid facts, machine and core_address in pane JSON.
split_out="$(run_herdr pane split --pane "$W" --direction right --on "$PC" --cwd "$BASE")" \
  || fail "pane split --on $PC failed: $split_out"
P="$(python3 -I -c 'import json,sys; print(json.loads(sys.argv[1])["result"]["pane"]["pane_id"])' "$split_out")"
ADDR="$(pane_field "$P" core_address)"
[[ "$(pane_field "$P" machine)" == "$PC" ]] || fail "pane JSON machine is $(pane_field "$P" machine)"
[[ "$ADDR" == "$PC/"t* ]] || fail "pane JSON core_address is $ADDR"
[[ -z "$(pane_field "$W" machine)" && "$(pane_field "$W" core_address)" =~ ^t[0-9]+$ ]] \
  || fail "the local pane has a machine field or no local address"
far_terminal "$ADDR" | python3 -I -c 'import json,sys
t = json.load(sys.stdin)
label, core_id = sys.argv[1], sys.argv[2]
assert t["labels"]["herdr.session"] == f"{label}@{core_id}", t["labels"]
assert t["labels"]["herdr.pane_id"] == sys.argv[3], t["labels"]
names = ["HERDR_ENV", "HERDR_PANE_ID", "HERDR_TAB_ID", "HERDR_WORKSPACE_ID", "HERDR_SOCKET_PATH", "HERDR_CLIENT_SOCKET_PATH"]
want = ["/usr/bin/env"] + [x for n in names for x in ("-u", n)] + ["--"]
assert t["argv"][:len(want)] == want, t["argv"]' "$CORE_SESSION" "$CORE_ID" "$P" \
  || fail "the far terminal's labels or argv are not D5's"
send_line "$P" "env | cut -d= -f1 | grep '^HERDR_' > $BASE/v9.names; echo \$\$ > $BASE/v9.pid; pwd > $BASE/v9.cwd"
wait_secs "the far shell's variable names" 15 file_has_line "$BASE/v9.pid"
sleep 0.3
if [[ -s "$BASE/v9.names" ]]; then fail "the far program saw herdr variables: $(tr '\n' ' ' < "$BASE/v9.names")"; fi
[[ "$(cat "$BASE/v9.cwd")" == "$(cd "$BASE" && pwd -P)" || "$(cat "$BASE/v9.cwd")" == "$BASE" ]] \
  || fail "the far shell started in $(cat "$BASE/v9.cwd"), not --cwd $BASE"
FAR_SHELL_PID="$(cat "$BASE/v9.pid")"
# The far pid is a live process on this Mac: a pid fact taken from it would
# show here.
api "{\"id\":\"pc\",\"method\":\"pane.process_info\",\"params\":{\"pane_id\":\"$P\"}}" > "$BASE/v9.proc" 2>&1 || true
if command grep -q "\"pid\": *$FAR_SHELL_PID\b\|\"pid\":$FAR_SHELL_PID[,}]" "$BASE/v9.proc"; then
  fail "the pane's process facts name the far pid $FAR_SHELL_PID: $(cat "$BASE/v9.proc")"
fi
[[ -z "$(pane_field "$P" foreground_cwd)" ]] || fail "the pane has a foreground_cwd from a local process"
# V31 pane part: a far-side `herdr pane report-agent` with no target finds
# no pane (no HERDR_PANE_ID) and reports to none of ours.
send_line "$P" "$HERDR_BIN pane report-agent --source v31 --agent claude --state blocked >$BASE/v31.out 2>&1; echo \$? > $BASE/v31.code"
wait_secs "the far report-agent" 15 file_has_line "$BASE/v31.code"
[[ "$(cat "$BASE/v31.code")" != 0 ]] || fail "a far report-agent with no target succeeded: $(cat "$BASE/v31.out")"
[[ "$(pane_field "$P" agent_status)" != blocked && "$(pane_field "$W" agent_status)" != blocked ]] \
  || fail "the far report reached one of our panes"
echo "ok: V9 $P runs on $PC as $ADDR, label $CORE_SESSION@$CORE_ID, argv wrapped, no HERDR_* in the program, no pid facts; V31 the far report-agent found no pane"

# --- V10: the machine's own herdr (same session label, same core session
# name) claims nothing, lists no orphan and kills nothing.
run_far_herdr server >/dev/null 2>&1 &
far_up() { [[ -S "$FAR_HERDR_SOCKET" ]]; }
wait_secs "the far herdr's socket" 20 far_up
sleep 3
far_log="$(cat "$PC_CONFIG"/herdr-dev/sessions/"$SESSION"/*.log 2>/dev/null || true)"
far_addr="${ADDR#"$PC"/}"
if printf '%s\n' "$far_log" | command grep -E "address=$far_addr\b|terminal $far_addr\b" | command grep -Eiq "orphan|has no pane|reattach|kill"; then
  fail "the far herdr acted on our pane's terminal $far_addr"
fi
if printf '%s\n' "$far_log" | command grep -q "without a pane"; then fail "the far herdr listed an orphan"; fi
run_far_herdr server stop >/dev/null 2>&1 || true
far_down() { [[ ! -S "$FAR_HERDR_SOCKET" ]]; }
wait_secs "the far herdr stopped" 20 far_down
far_live "$ADDR" || fail "our pane's far terminal ended with the far herdr"
send_line "$P" "echo v10 > $BASE/v10.ok"
wait_secs "input after the far herdr" 10 file_has_line "$BASE/v10.ok"
echo "ok: V10 the far herdr (label $CORE_SESSION) left $far_addr alone"

# --- V12: close is terminal.kill only: the far core and its other
# terminals run on, and a job the program left behind is not swept.
hc_scratch run --bg --no-parent --no-notify --on "$PC" sh -c "echo \$\$ > $BASE/v12.other; exec sleep 600" >/dev/null \
  || fail "the other far terminal did not start"
wait_secs "the other far terminal" 10 file_has_line "$BASE/v12.other"
send_line "$P" "nohup sleep 600 >/dev/null 2>&1 & echo \$! > $BASE/v12.bg"
wait_secs "the far background job" 10 file_has_line "$BASE/v12.bg"
api "{\"id\":\"pc\",\"method\":\"pane.close\",\"params\":{\"pane_id\":\"$P\"}}" >/dev/null || fail "pane.close failed"
gone_far() { ! far_live "$ADDR"; }
wait_secs "the far terminal ended" 10 gone_far
alive "$(cat "$BASE/v12.other")" || fail "closing the pane ended another far terminal"
alive "$(cat "$BASE/v12.bg")" || fail "closing the pane swept the far program's background job"
[[ -S "$PC_SOCK" ]] || fail "the far core went away"
kill "$(cat "$BASE/v12.bg")" "$(cat "$BASE/v12.other")" 2>/dev/null || true
echo "ok: V12 closing the pane ended $ADDR only; the far core, its other terminal and the program's background job run on"

# A fresh pane on the machine for the waiting cases.
start_pane() {
  local out
  out="$(run_herdr pane split --pane "$W" --direction down --on "$PC")" || fail "pane split --on $PC failed: $out"
  P="$(python3 -I -c 'import json,sys; print(json.loads(sys.argv[1])["result"]["pane"]["pane_id"])' "$out")"
  ADDR="$(pane_field "$P" core_address)"
  rm -f "$BASE/far.pid"
  send_line "$P" "echo \$\$ > $BASE/far.pid"
  wait_secs "the far program's pid" 10 file_has_line "$BASE/far.pid"
  FAR_PID="$(cat "$BASE/far.pid")"
}
start_pane
far_terminals() { core_rpc terminal.list '{"all":true}' | python3 -I -c 'import json,sys; print(sum(1 for t in json.load(sys.stdin)["terminals"] if t["address"].startswith(sys.argv[1] + "/")))' "$PC"; }

# --- V6: restart and live handoff while the machine is away: the pane
# waits as machine_unreachable, no dial, no far program starts; it attaches
# again to the same far program.
# Counted while connected: an unreachable machine's terminals leave the list (P5).
before="$(far_terminals)"
touch "$BASE/pc.down"
drop_link
wait_secs "$PC unreachable" 15 pc_is unreachable
run_herdr server restart --yes >"$BASE/pc.restart" 2>&1 || fail "restart failed: $(tail -n 5 "$BASE/pc.restart")"
wait_until "server after restart" socket_up
wait_secs "the pane waiting as machine_unreachable" 15 cause_is "$P" machine_unreachable
run_herdr server live-handoff --import-exe "$HERDR_BIN" >/dev/null || fail "live handoff did not complete"
wait_until "server after handoff" socket_up
wait_secs "the pane still waiting after the handoff" 15 cause_is "$P" machine_unreachable
[[ "$(pane_field "$P" core_address)" == "$ADDR" ]] || fail "the waiting pane lost its address"
alive "$FAR_PID" || fail "the far program ended while the machine was away"
rm -f "$BASE/pc.down"
hc_scratch machine connect "$PC" >/dev/null 2>&1 || true
wait_secs "$PC connected again" 30 pc_is connected
wait_secs "the pane attached again" 15 not_waiting "$P"
[[ "$(far_terminals)" == "$before" ]] || fail "a far program started for the waiting pane"
rm -f "$BASE/far.again"
send_line "$P" "echo \$\$ > $BASE/far.again"
wait_secs "input after the reattach" 10 file_has_line "$BASE/far.again"
[[ "$(cat "$BASE/far.again")" == "$FAR_PID" ]] || fail "the pane attached to another program: $(cat "$BASE/far.again") not $FAR_PID"
echo "ok: V6 (unreachable) a restart and a live handoff kept $P waiting as machine_unreachable; it attached again to far pid $FAR_PID"

# --- V6 dormant and V4's server part: herdr gates the machine (its folder
# has a target.json) and no lease is in force, so it is dormant: a restart
# waits as machine_dormant and the poll reads nothing; a lease (a person's
# act) lets the poll find it and the pane attach.
mkdir -p "$PC_DIR/leases"
chmod 700 "$PC_ROOT" "$PC_DIR" "$PC_DIR/leases" 2>/dev/null || true
printf '{}\n' > "$PC_DIR/target.json"
touch "$BASE/pc.down"
drop_link
wait_secs "$PC unreachable" 15 pc_is unreachable
run_herdr server restart --yes >"$BASE/pc.restart" 2>&1 || fail "restart failed: $(tail -n 5 "$BASE/pc.restart")"
wait_until "server after restart" socket_up
wait_secs "the pane waiting as machine_dormant" 15 cause_is "$P" machine_dormant
rm -f "$BASE/pc.down"
hc_scratch machine connect "$PC" >/dev/null 2>&1 || true
wait_secs "$PC connected" 30 pc_is connected
sleep 7
cause_is "$P" machine_dormant || fail "the poll read a dormant machine ($(waiting_cause "$P"))"
write_lease 120
wait_secs "the pane attached once a lease allowed it" 12 not_waiting "$P"
rm -f "$BASE/far.again"
send_line "$P" "echo \$\$ > $BASE/far.again"
wait_secs "input after the reattach" 10 file_has_line "$BASE/far.again"
[[ "$(cat "$BASE/far.again")" == "$FAR_PID" ]] || fail "the pane attached to another program"
echo "ok: V6 (dormant) a restart kept $P waiting as machine_dormant; the poll read nothing until a lease, then it attached to far pid $FAR_PID"

# V4's server part: the link drops and core's retries reach their 30 s cap;
# with a lease in force (the client writes one for a person's key, B.2),
# input to the attached pane is not sent and posts the server's own wake:
# the machine is connected again well before core's next retry.
write_lease 600
v4_try() {
  touch "$BASE/pc.down"
  drop_link
  wait_secs "$PC unreachable" 15 pc_is unreachable
  sleep 70
  rm -f "$BASE/pc.down"
  sleep 6
  pc_is unreachable
}
v4_ready=0
for _ in 1 2 3; do
  if v4_try; then v4_ready=1; break; fi
  echo "V4: core's own retry reconnected $PC first; dropping again"
done
[[ "$v4_ready" == 1 ]] || fail "core's own retries kept reconnecting $PC; cannot time the wake"
rm -f "$BASE/v4.dropped"
key_at=$SECONDS
send_line "$P" "echo dropped > $BASE/v4.dropped" 2>/dev/null || true
wait_secs "$PC connected by the server's wake" 5 pc_is connected
woke_in=$((SECONDS - key_at))
sleep 1
[[ ! -e "$BASE/v4.dropped" ]] || fail "input typed while $PC was away was sent"
send_line "$P" "echo after > $BASE/v4.after"
wait_secs "input after the wake" 10 file_has_line "$BASE/v4.after"
echo "ok: V4 (server part) a key while $PC was away was not sent; the server's wake connected it within ${woke_in}s"

# --- V11: a long drop with a lease kept: the pane keeps its place and its
# far program; input works after.
write_lease "$((V11_SECS + 600))"
touch "$BASE/pc.down"
drop_link
wait_secs "$PC unreachable" 15 pc_is unreachable
echo "V11: $PC refused for ${V11_SECS}s"
sleep "$V11_SECS"
alive "$FAR_PID" || fail "V11: the far program ended during the drop"
rm -f "$BASE/pc.down"
hc_scratch machine connect "$PC" >/dev/null 2>&1 || true
wait_secs "$PC connected after the drop" 40 pc_is connected
wait_secs "the pane back after the drop" 75 not_waiting "$P"
# The first key after the drop may find the machine still marked away
# until the next poll (at most 5 s): it is dropped and posts a wake, so the
# test types again, as a person would.
rm -f "$BASE/far.again"
typed_again() {
  file_has_line "$BASE/far.again" && return 0
  send_line "$P" "echo \$\$ > $BASE/far.again" 2>/dev/null || true
  sleep 1
  file_has_line "$BASE/far.again"
}
wait_secs "input after the long drop" 75 typed_again
[[ "$(cat "$BASE/far.again")" == "$FAR_PID" ]] || fail "V11: another program after the drop"
echo "ok: V11 after ${V11_SECS}s away $P kept its place and far pid $FAR_PID"

api "{\"id\":\"pc\",\"method\":\"pane.close\",\"params\":{\"pane_id\":\"$P\"}}" >/dev/null || true
link_cleanup
link_cleanup() { :; }
echo "PR C machine pane checks passed"
