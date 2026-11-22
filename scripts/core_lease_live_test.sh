# Sourced by scripts/core_machine_live_test.sh after core_link_live_test.sh,
# with its scratch cores, helpers and trap (openspec/changes/core-phase5,
# PR B: leases and dormancy).
#
# V2 (a lease's end closes the link), V7 (a core start while dormant dials
# nothing), V3 (dormant means zero link bytes), V38 (a refusing far side
# costs at most 2 dials) beside V3 on a second far core, then a TUI client
# in a pty (its lease at start, the wake through its dialer, one write a
# minute) and V33's wake part with `herdr machine wake`. Every time
# constant that is shortened is shortened through the lease files (the
# interface herdr reads), never a test-only setting; the waits of V3 and
# V38 are the design's 5 and 10 min unless V3_SECS or V38_SECS say less.
#
# Nothing dials another host: a fake ssh to the far scratch core, and real
# OpenSSH only to a loopback server on 127.0.0.1.

[[ -n "${LINK_ROOT:-}" && -n "${RUN_TAG:-}" ]] || { echo "source this after core_link_live_test.sh" >&2; exit 1; }

L="$RUN_TAG-lease"     # fake ssh to the far core: V2, V7, V3, the client, wake
V="$RUN_TAG-spent"     # real OpenSSH to a loopback server, a second far core: V38
V3_SECS="${V3_SECS:-300}"
V38_SECS="${V38_SECS:-600}"
FAR2_SESSION="$FAR_SESSION-2"
FAR2_RUNTIME_DIR="$BASE/f2r"
FAR2_STATE_DIR="$BASE/f2s"
FAR2_SOCK="$FAR2_RUNTIME_DIR/herdr-core/$FAR2_SESSION.sock"
SSHD2="$BASE/sshd2"
V38_PID=""
CLIENT_PID=""

hc_far2() {
  env -u HC_SOCKET XDG_RUNTIME_DIR="$FAR2_RUNTIME_DIR" XDG_STATE_HOME="$FAR2_STATE_DIR" \
    HC_SESSION="$FAR2_SESSION" hc "$@"
}

# The core-link part's cleanup, then this part's.
eval "$(declare -f link_cleanup | sed '1s/link_cleanup/link_cleanup_a/')"
link_cleanup() {
  set +e
  [[ -n "$V38_PID" ]] && kill "$V38_PID" 2>/dev/null
  [[ -n "$CLIENT_PID" ]] && kill -9 "$CLIENT_PID" 2>/dev/null
  [[ -s "$SSHD2/server.pid" ]] && kill "$(cat "$SSHD2/server.pid")" 2>/dev/null
  [[ -s "$SSHD2/uv.pid" ]] && kill "$(cat "$SSHD2/uv.pid")" 2>/dev/null
  for name in "$L" "$V"; do
    [[ -S "$LOCAL_SOCK" ]] && hc_scratch machine rm "$name" >/dev/null 2>&1
  done
  # The test client's own lease in any machine folder it wrote one to.
  [[ -n "$CLIENT_PID" ]] && rm -f "$LINK_ROOT"/*/leases/"client-$CLIENT_PID"
  link_cleanup_a
  [[ -S "$FAR2_SOCK" ]] && hc_far2 server stop >/dev/null 2>&1
  for name in "$L" "$V"; do
    case "$name" in t[0-9]*-*) rm -rf "${LINK_ROOT:?}/$name" ;; esac
  done
}

# The STARTs of $1's link log at or after time $2, as JSON: POSIX and
# Windows line times, how many were not refused, the EXECs (ssh started),
# the Windows line's lag behind its POSIX line, the gaps between POSIX
# STARTs after time $3 (default $2).
starts_report() {
  python3 -I - "$(link_log "$1")" "$2" "${3:-$2}" <<'PY'
import json, sys
rows = []
for line in open(sys.argv[1]):
    parts = line.rstrip("\n").split(" ", 3)
    if len(parts) < 3:
        continue
    try:
        at = float(parts[0])
    except ValueError:
        continue
    rows.append((at, parts[1], parts[2], parts[3] if len(parts) > 3 else ""))
since, settled = float(sys.argv[2]), float(sys.argv[3])
out = {"posix": [], "windows": [], "unrefused": 0, "exec": 0}
for at, pid, kind, rest in rows:
    if at < since or kind != "START":
        continue
    out["windows" if 'line="cmd' in rest else "posix"].append(at)
    after = next((r for r in rows if r[1] == pid and r[0] >= at and r[2] != "START"), None)
    if after is None or after[2] != "REFUSE":
        out["unrefused"] += 1
out["exec"] = sum(1 for r in rows if r[0] >= since and r[2] == "EXEC")
lags = [w - max(p for p in out["posix"] if p <= w) for w in out["windows"] if any(p <= w for p in out["posix"])]
out["max_lag"] = max(lags) if lags else None
late = [p for p in out["posix"] if p >= settled]
out["gaps"] = [b - a for a, b in zip(late, late[1:])]
print(json.dumps(out))
PY
}
field() { python3 -I -c 'import json,sys; v = json.loads(sys.argv[1])[sys.argv[2]]; print(len(v) if isinstance(v, list) else v)' "$1" "$2"; }
min_gap() { python3 -I -c 'import json,sys; g = json.loads(sys.argv[1])["gaps"]; print(f"{min(g):.2f}" if g else "none")' "$1"; }
gap_text() { [[ "$1" == none ]] && echo "no pair after the cap to compare" || echo "pairs after the cap at least ${1}s apart"; }
fake_bytes_since() { awk -v t="$1" -v m="$2" '$1 >= t && $2 == m' "$BASE/bytes.log" 2>/dev/null | wc -l | tr -d ' '; }
ssh_starts_since() { awk -v t="$1" -v m="machine=$2" '$1 >= t && $2 == "SSH" && index($0, m)' "$SSH_LOG" | wc -l | tr -d ' '; }
mtime() { python3 -I -c 'import os,sys; print(os.stat(sys.argv[1]).st_mtime)' "$1"; }
ge() { le "$2" "$1"; }

# --- L: a machine through the fake ssh, as a person adds it
echo relay > "$SSH_MODE"
herdr_tty SSH_AUTH_SOCK="$AGENT_SOCK" PATH="$BASE/bin:$PATH" \
  "$HERDR_BIN" --session "$SESSION" machine add --core lease-target --name "$L" --remote-session "$FAR_SESSION" \
  >"$BASE/add.out" 2>&1 || fail "machine add --core $L failed: $(tail -n 5 "$BASE/add.out")"
wait_for "$L connected" 20 state_is "$L" connected
wait_for "$L's link settles" 15 more_in_log "$L" "SETTLED" 0

# --- V2: a lease with a 20 s expiry; its end closes the link within 5 s
# The add's 30 min manual lease is the only one; this rewrites it.
v2_start="$(now)"
write_lease "$L" manual "$(plus 20)" "$v2_start"
wait_for "the lease end" 35 more_in_log "$L" "LEASE END" 0
v2_end="$(log_time "$L" "LEASE END")"
v2_took="$(python3 -I -c 'import sys; print(f"{float(sys.argv[2]) - float(sys.argv[1]):.2f}")' "$v2_start" "$v2_end")"
ge "$v2_took" 20 && le "$v2_took" 25.5 || fail "the link ended ${v2_took}s after a 20 s lease began"
wait_for "$L unreachable" 15 state_is "$L" unreachable
for pid in $(grep " EXEC " "$(link_log "$L")" | sed 's/.* pid=\([0-9]*\) .*/\1/'); do
  ours_alive "$pid" && fail "the link's ssh $pid outlived its lease"
done
run_herdr machine status --core "$L" --json | python3 -I -c 'import json,sys
row = json.load(sys.stdin)[0]
assert row["gate"] == "dormant" and row["state"] != "connected", row' || fail "machine status does not say dormant"
echo "ok: V2 the link ended ${v2_took}s after a 20 s lease began; $L unreachable; status says dormant"

# --- V7: a core start while dormant dials nothing
v7_start="$(now)"
v7_ssh="$(ssh_starts lease-target)"
v7_starts="$(log_count "$L" START)"
hc_scratch server restart >/dev/null 2>&1 || fail "hc server restart failed"
wait_for "the local core back" 20 test -S "$LOCAL_SOCK"
wait_for "core's start dials $L" 20 more_in_log "$L" "START" "$v7_starts"
sleep 8
report="$(starts_report "$L" "$v7_start")"
[[ "$(field "$report" posix)" -ge 1 && "$(field "$report" windows)" -ge 1 ]] || fail "the core start did not dial both lines: $report"
[[ "$(field "$report" unrefused)" == 0 && "$(field "$report" exec)" == 0 ]] || fail "a dial ran at the core start: $report"
[[ "$(ssh_starts lease-target)" == "$v7_ssh" ]] || fail "the core start ran ssh"
[[ "$(machine_state "$L")" != connected ]] || fail "$L connected while dormant"
echo "ok: V7 a core restart while dormant: $(field "$report" posix) POSIX and $(field "$report" windows) Windows STARTs, all refused, no ssh"

# --- V38, in the background beside V3: real OpenSSH to a second loopback
# server, on a second far core, that drops every connection at its auth
# request (a far sshd going away; an explicit `Permission denied` is
# auth_failed, V8, which closes until login). A client lease renewed every
# 30 s by a timer keeps its act time: no new input.
v38() {
  set -euo pipefail
  mkdir -p "$SSHD2" "$FAR2_RUNTIME_DIR" "$FAR2_STATE_DIR"
  hc_far2 server start >/dev/null 2>&1 || fail "V38: could not start the second far core"
  wait_for "V38: the second far core's socket" 10 test -S "$FAR2_SOCK"
  local port
  port="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
  printf '[127.0.0.1]:%s %s\n' "$port" "$(cut -d' ' -f1,2 "$BASE/sshd/hkey.pub")" > "$SSHD2/known_hosts"
  cat > "$SSHD2/config" <<CFG
Host loopback2
  HostName 127.0.0.1
  Port $port
  User $(id -un)
  IdentityFile $BASE/sshd/ckey
  IdentitiesOnly yes
  IdentityAgent none
  UserKnownHostsFile $SSHD2/known_hosts
  GlobalKnownHostsFile /dev/null
CFG
  echo accept > "$SSHD2/mode"
  cat > "$SSHD2/server.py" <<SERVER
import os, socket, subprocess, threading, time
import paramiko
LOG = open("$SSHD2/auth.log", "a", buffering=1)
client_key = paramiko.Ed25519Key.from_private_key_file("$BASE/sshd/ckey")
host_key = paramiko.Ed25519Key.from_private_key_file("$BASE/sshd/hkey")
ENV = {"HOME": os.environ["HOME"], "USER": os.environ.get("USER", ""), "PATH": "/usr/bin:/bin:" + os.environ["HOME"] + "/.local/bin",
       "XDG_RUNTIME_DIR": "$FAR2_RUNTIME_DIR", "XDG_STATE_HOME": "$FAR2_STATE_DIR"}
class Server(paramiko.ServerInterface):
    def __init__(self, transport):
        self.transport = transport; self.counted = False
    def check_auth_publickey(self, username, key):
        if open("$SSHD2/mode").read().strip() == "refuse":
            if not self.counted:
                self.counted = True; LOG.write(f"{time.time():.3f} AUTH refused\n")
            self.transport.close()
            return paramiko.AUTH_FAILED
        ok = key.asbytes() == client_key.asbytes()
        if ok and not self.counted:
            self.counted = True; LOG.write(f"{time.time():.3f} AUTH ok\n")
        return paramiko.AUTH_SUCCESSFUL if ok else paramiko.AUTH_FAILED
    def get_allowed_auths(self, username): return "publickey"
    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
    def check_channel_exec_request(self, channel, command):
        proc = subprocess.Popen(["sh", "-c", command.decode()], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV, start_new_session=True)
        def out():
            while (b := os.read(proc.stdout.fileno(), 65536)): channel.sendall(b)
            channel.send_exit_status(proc.wait()); channel.close()
        def err():
            while (b := os.read(proc.stderr.fileno(), 65536)): channel.sendall_stderr(b)
        def inp():
            try:
                while (b := channel.recv(65536)): proc.stdin.write(b); proc.stdin.flush()
            except Exception: pass
            try: proc.stdin.close()
            except Exception: pass
            try: os.killpg(proc.pid, 9)
            except Exception: pass
        for f in (out, err, inp): threading.Thread(target=f, daemon=True).start()
        return True
def handle(sock):
    LOG.write(f"{time.time():.3f} CONN\n")
    t = paramiko.Transport(sock); t.add_server_key(host_key); t.start_server(server=Server(t))
    while t.is_active(): time.sleep(0.5)
ls = socket.socket(); ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); ls.bind(("127.0.0.1", $port)); ls.listen(8)
open("$SSHD2/server.pid", "w").write(str(os.getpid()))
LOG.write(f"{time.time():.3f} LISTEN\n")
while True:
    s, _ = ls.accept(); threading.Thread(target=handle, args=(s,), daemon=True).start()
SERVER
  (cd "$SSHD2" && exec uv run --no-project --quiet --with paramiko python -I "$SSHD2/server.py") >"$SSHD2/server.out" 2>&1 &
  echo $! > "$SSHD2/uv.pid"
  wait_for "V38: the loopback server listens" 60 grep -qs LISTEN "$SSHD2/auth.log"

  herdr_tty SSH_AUTH_SOCK= PATH="/usr/bin:/bin:/usr/sbin:/sbin" "$HERDR_BIN" --session "$SESSION" \
    machine add --core loopback2 --name "$V" --remote-session "$FAR2_SESSION" --ssh-config "$SSHD2/config" \
    >"$SSHD2/add.out" 2>&1 || fail "V38: machine add --core $V failed: $(tail -n 5 "$SSHD2/add.out")"
  wait_for "V38: $V connected" 20 state_is "$V" connected
  wait_for "V38: $V settled" 15 more_in_log "$V" "SETTLED" 0
  # Dormant, then the far side refuses.
  write_lease "$V" manual "$(plus -1)" "$(now)"
  wait_for "V38: the lease end" 15 more_in_log "$V" "LEASE END" 0
  wait_for "V38: $V unreachable" 15 state_is "$V" unreachable
  echo refuse > "$SSHD2/mode"
  local act holder="client-$BASHPID" t0 refused_before execs auths
  act="$(now)"
  t0="$act"
  refused_before="$(grep -c "AUTH refused" "$SSHD2/auth.log" || true)"
  write_lease "$V" "$holder" "$(plus 60)" "$act"
  while python3 -I -c 'import sys,time; sys.exit(0 if time.time() - float(sys.argv[1]) < float(sys.argv[2]) else 1)' "$t0" "$V38_SECS"; do
    sleep 30
    write_lease "$V" "$holder" "$(plus 60)" "$act"
  done
  local report
  report="$(starts_report "$V" "$t0")"
  execs="$(field "$report" exec)"
  auths=$(( $(grep -c "AUTH refused" "$SSHD2/auth.log" || true) - refused_before ))
  echo "V38: ${V38_SECS}s with a lease and no new act: $execs ssh starts, $auths refused auths, $(field "$report" posix) STARTs, gate $(gate_class "$V")"
  (( execs <= 2 && auths <= 2 )) || fail "V38: a refusing far side cost $execs dials ($auths auths) in ${V38_SECS}s"
  [[ "$(gate_class "$V")" == retry_spent ]] || fail "V38: the gate is $(gate_class "$V"), not retry_spent"
  # One key in a far pane, as the client handles it: its lease with that
  # input's time, then a wake through its dialer (machine.connect). Core's
  # own retries meanwhile stay refused.
  local key
  key="$(now)"
  write_lease "$V" "$holder" "$(plus 120)" "$key"
  connect_now "$V" >/dev/null
  more_execs() { [[ "$(field "$(starts_report "$V" "$t0")" exec)" -gt "$1" ]]; }
  wait_for "V38: the key's dial" 25 more_execs "$execs"
  sleep 40
  report="$(starts_report "$V" "$t0")"
  local after_auths
  after_auths=$(( $(grep -c "AUTH refused" "$SSHD2/auth.log" || true) - refused_before ))
  [[ "$(field "$report" exec)" == $((execs + 1)) && "$after_auths" == $((auths + 1)) ]] \
    || fail "V38: the key gave $(( $(field "$report" exec) - execs )) dials ($((after_auths - auths)) auths), not exactly one"
  [[ "$(gate_class "$V")" == retry_spent ]] || fail "V38: after the key's dial the gate is $(gate_class "$V")"
  python3 -I -c 'import json,sys; g = json.load(open(sys.argv[1])); sys.exit(0 if g["at"] > float(sys.argv[2]) else 1)' \
    "$LINK_ROOT/$V/gate.json" "$key" || fail "V38: retry_spent was not recorded after the key"
  echo "ok: V38 ${V38_SECS}s of refused auth: $execs dials, gate retry_spent; one key and its wake: exactly one more dial, then none in 40 s"
  run_herdr machine rm --core "$V" >/dev/null 2>&1 || fail "V38: machine rm --core $V failed"
  kill "$(cat "$SSHD2/server.pid")" 2>/dev/null || true
  hc_far2 server stop >/dev/null 2>&1 || true
}
v38 >"$BASE/v38.log" 2>&1 &
V38_PID=$!

# --- V3: dormant means zero link bytes. A client lease that ends 30 s
# later, a wake as the dialer makes it, then V3_SECS of dormancy.
v3_act="$(now)"
v3_end_before="$(log_count "$L" "LEASE END")"
write_lease "$L" "client-$$" "$(plus 30)" "$v3_act"
read -r took state < <(connect_now "$L")
[[ "$state" == connected ]] || fail "V3: the wake answered $state"
lease_end_or_dump() {
  more_in_log "$L" "LEASE END" "$v3_end_before" && return 0
  python3 -I -c 'import sys,time; sys.exit(0 if time.time() - float(sys.argv[1]) < 45 else 1)' "$v3_act" && return 1
  { echo "V3 debug: now $(now), leases:"; for f in "$LINK_ROOT/$L"/leases/*; do echo "  $(basename "$f") $(cat "$f")"; done
    tail -n 12 "$(link_log "$L")"; } >&2
  return 0
}
wait_for "V3: the lease end" 60 lease_end_or_dump
more_in_log "$L" "LEASE END" "$v3_end_before" || fail "V3: no lease end 45 s after a 30 s lease"
v3_dormant="$(log_time "$L" "LEASE END" $((v3_end_before + 1)))"
polls="$(awk -v a="$v3_act" -v b="$v3_dormant" '$1 >= a && $1 < b && $2 == "lease-target" && $3 == "up"' "$BASE/bytes.log" | wc -l | tr -d ' ')"
(( polls >= 5 )) || fail "V3: only $polls polls went up while the link was active"
echo "  V3: active $(python3 -I -c 'import sys; print(f"{float(sys.argv[2]) - float(sys.argv[1]):.1f}")' "$v3_act" "$v3_dormant")s with $polls sends up; dormant from $v3_dormant; waiting ${V3_SECS}s"
sleep "$V3_SECS"
quiet_from="$(python3 -I -c 'import sys; print(float(sys.argv[1]) + 1.5)' "$v3_dormant")"
capped_from="$(python3 -I -c 'import sys; print(float(sys.argv[1]) + 70)' "$v3_dormant")"
report="$(starts_report "$L" "$quiet_from" "$capped_from")"
[[ "$(fake_bytes_since "$quiet_from" lease-target)" == 0 ]] || fail "V3: link bytes after dormancy: $(fake_bytes_since "$quiet_from" lease-target) lines"
[[ "$(ssh_starts_since "$quiet_from" lease-target)" == 0 ]] || fail "V3: ssh started while dormant"
[[ "$(field "$report" unrefused)" == 0 && "$(field "$report" exec)" == 0 ]] || fail "V3: a dial ran while dormant: $report"
posix="$(field "$report" posix)"; windows="$(field "$report" windows)"
(( posix >= 3 && windows >= posix - 1 && windows <= posix )) || fail "V3: STARTs not in POSIX and Windows pairs: $report"
lag="$(python3 -I -c 'import json,sys; print(f"{json.loads(sys.argv[1])["max_lag"]:.3f}")' "$report")"
le "$lag" 0.25 || fail "V3: the Windows line came ${lag}s after its POSIX line"
gap="$(min_gap "$report")"
[[ "$gap" == none ]] || ge "$gap" 25 || fail "V3: refused pairs ${gap}s apart once the backoff reached its cap"
echo "ok: V3 ${V3_SECS}s dormant: 0 link bytes, 0 ssh starts; $posix refused pairs (Windows lag <= ${lag}s), $(gap_text "$gap")"

wait "$V38_PID" || { cat "$BASE/v38.log" >&2; V38_PID=""; fail "V38 failed (log above)"; }
V38_PID=""
cat "$BASE/v38.log"

# --- B.2, B.5: a TUI client started by a person's command writes its lease
# at start, before its dialer's connect; a dormant machine connects at once,
# not at core's next retry (30 s apart by now); input within the minute
# writes nothing more.
cat > "$BASE/client_pty.py" <<'PY'
import fcntl, os, pty, select, signal, struct, sys, termios, time
hold, keys_from = float(sys.argv[1]), float(sys.argv[2])
pid, fd = pty.fork()
if pid == 0:
    os.execvp(sys.argv[3], sys.argv[3:])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
print(pid, flush=True)
start = time.time(); next_key = start + keys_from
while time.time() - start < hold:
    ready, _, _ = select.select([fd], [], [], 0.2)
    if ready:
        try: os.read(fd, 65536)
        except OSError: break
    if time.time() >= next_key:
        os.write(fd, b"x"); next_key += 0.5
os.kill(pid, signal.SIGTERM)
time.sleep(1)
try: os.kill(pid, signal.SIGKILL)
except ProcessLookupError: pass
PY
last_refused="$(log_count "$L" "REFUSE")"
wait_for "a refused retry to start the client after" 40 more_in_log "$L" "REFUSE" "$last_refused"
client_start="$(now)"
starts_before="$(log_count "$L" "START")"
python3 -I "$BASE/client_pty.py" 12 4 env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH \
  -u HERDR_SESSION -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION \
  -u HERDR_CLIENT_REEXEC -u CLAUDECODE TERM=xterm-256color \
  XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" SHELL=/bin/sh \
  "$HERDR_BIN" --session "$SESSION" >"$BASE/client.out" 2>/dev/null &
client_runner=$!
wait_for "the client's pid" 5 test -s "$BASE/client.out"
CLIENT_PID="$(head -n 1 "$BASE/client.out")"
CLIENT_LEASE="$LINK_ROOT/$L/leases/client-$CLIENT_PID"
wait_for "the client's lease" 5 test -f "$CLIENT_LEASE"
wait_for "$L connected by the client's wake" 4 state_is "$L" connected
connected_after="$(since "$client_start")"
le "$(mtime "$CLIENT_LEASE")" "$(log_time "$L" START $((starts_before + 1)))" \
  || fail "the client's lease came after the dial it led to"
lease_text="$(cat "$CLIENT_LEASE")"
sleep 8   # keys from 4 s on, every 0.5 s
[[ "$(cat "$CLIENT_LEASE")" == "$lease_text" ]] || fail "the client wrote its lease again within the minute"
wait "$client_runner" || true
ends="$(log_count "$L" "LEASE END")"
rm -f "$LINK_ROOT"/*/leases/"client-$CLIENT_PID"
echo "ok: B.2/B.5 the client's lease at start preceded its dial; $L connected ${connected_after}s after the client started; keys within the minute wrote nothing"

# --- V33 (wake part) and V35's reopening: herdr machine wake from dormant
wait_for "$L dormant after the client" 15 more_in_log "$L" "LEASE END" "$ends"
wait_for "$L unreachable" 15 state_is "$L" unreachable
wake_start="$(now)"
starts_before="$(log_count "$L" "START")"
run_herdr machine wake "$L" --for 20s >"$BASE/wake.out" 2>&1 || fail "machine wake failed: $(cat "$BASE/wake.out")"
grep -q "^$L: connected$" "$BASE/wake.out" || fail "machine wake did not connect: $(cat "$BASE/wake.out")"
manual_mtime="$(mtime "$LINK_ROOT/$L/leases/manual")"
ge "$manual_mtime" "$wake_start" && le "$manual_mtime" "$(log_time "$L" START $((starts_before + 1)))" \
  || fail "the wake's lease did not come before its dial"
echo "ok: V33 machine wake wrote its lease before the dial and connected"
ends="$(log_count "$L" "LEASE END")"
wait_for "the wake's 20 s lease ends" 30 more_in_log "$L" "LEASE END" "$ends"
wait_for "$L unreachable" 15 state_is "$L" unreachable
# A wake reopens far_core_down at once.
open_gate "$L"
hc_far server stop >/dev/null || fail "could not stop the far core"
wait_until "far socket gone" test ! -S "$FAR_SOCK"
code=0; run_herdr machine wake "$L" --for 60s >"$BASE/wake.out" 2>&1 || code=$?
[[ "$code" != 0 && "$(gate_class "$L")" == far_core_down ]] || fail "a wake with the far core stopped gave $code, gate $(gate_class "$L")"
hc_far server start >/dev/null 2>&1 || fail "could not start the far core"
run_herdr machine wake "$L" --for 20s >"$BASE/wake.out" 2>&1 || fail "the wake after the far core started failed: $(cat "$BASE/wake.out")"
grep -q "^$L: connected$" "$BASE/wake.out" && grep -q "WAKE cleared far_core_down" "$(link_log "$L")" \
  || fail "machine wake did not reopen far_core_down: $(cat "$BASE/wake.out")"
# The login classes are not a wake's to reopen.
printf '{"class":"auth_failed","at":%s}\n' "$(now)" > "$LINK_ROOT/$L/gate.json"
starts_before="$(log_count "$L" "START")"
code=0; run_herdr machine wake "$L" >"$BASE/wake.out" 2>&1 || code=$?
[[ "$code" == 1 ]] && grep -q "herdr machine login $L" "$BASE/wake.out" || fail "a wake over auth_failed did not refuse: $(cat "$BASE/wake.out")"
[[ "$(log_count "$L" "START")" == "$starts_before" ]] || fail "a refused wake dialled"
echo "ok: V35 machine wake reopened far_core_down and connected; a wake over auth_failed refuses and points to login"

run_herdr machine rm --core "$L" >/dev/null 2>&1 || fail "machine rm --core $L failed"
[[ ! -e "$LINK_ROOT/$L" && "$(machine_state "$L")" == absent ]] || fail "machine rm --core left $L"
none_left() {
  local pid
  for pid in $(logged_pids); do ours_alive "$pid" && return 1; done
  [[ ! -s "$SSHD2/server.pid" ]] || ! alive "$(cat "$SSHD2/server.pid")"
}
wait_for "every dial's processes to end" 10 none_left
echo "ok: leases part done; no ssh, watcher, client or loopback server left"
