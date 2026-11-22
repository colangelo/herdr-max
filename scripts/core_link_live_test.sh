# Sourced by scripts/core_machine_live_test.sh after its V26 part, with its
# scratch cores, helpers and trap (openspec/changes/core-phase5, PR A).
#
# `herdr core-link` against the far scratch core: a fake `ssh` first on PATH
# for V8, V27, V28, V31 (core-link part), V33 (add and login) and V35, and
# real OpenSSH to a loopback-only server for V34 and V36. Nothing dials
# another host. Leases are PR B: until then every machine counts as leased.
#
# core-link keeps its state under the passwd home, never HOME (F6), so the
# machines here get names unique to this run and their folders are removed
# at the end.

[[ -n "${BASE:-}" && -n "${FAR_SESSION:-}" ]] || { echo "source this from core_machine_live_test.sh" >&2; exit 1; }

PASSWD_HOME="$(python3 -I -c 'import os, pwd; print(pwd.getpwuid(os.getuid()).pw_dir)')"
LINK_ROOT="$PASSWD_HOME/.local/state/herdr-dev/core-machines"
RUN_TAG="t$$"
M="$RUN_TAG-fake"      # added through core, fake ssh
D="$RUN_TAG-direct"    # state only: core-link run by hand, fake ssh
R="$RUN_TAG-real"      # added through core, real OpenSSH to the loopback server
SSH_LOG="$BASE/ssh.log"
SSH_MODE="$BASE/ssh_mode"
AGENT_SOCK="$BASE/agent.sock"
LOCAL_SOCK="$RUNTIME_DIR/herdr-core/$CORE_SESSION.sock"
FAR_SOCK="$FAR_RUNTIME_DIR/herdr-core/$FAR_SESSION.sock"
SSHD_PID=""
UV_PID=""
PY="$(command -v python3)"

# A logged pid that is still one of this run's processes: its command line
# names this run's scratch folder or machines (never a recycled pid).
ours_alive() {
  local command
  command="$(ps -o command= -p "$1" 2>/dev/null)" || return 1
  [[ "$command" == *"$BASE"* || "$command" == *"$RUN_TAG"* ]]
}
logged_pids() {
  awk '{for (i = 1; i <= NF; i++) if ($i ~ /^pid=[0-9]+$/) print substr($i, 5)}' \
    "$SSH_LOG" "$LINK_ROOT"/"$RUN_TAG"-*/link.log 2>/dev/null | sort -u
}

link_cleanup() {
  set +e
  for name in "$M" "$D" "$R"; do
    [[ -S "$LOCAL_SOCK" ]] && hc_scratch machine rm "$name" >/dev/null 2>&1
  done
  # Every process this part started: ssh (fake or real) and watchers, by
  # the pids their logs recorded.
  for pid in $(logged_pids); do
    ours_alive "$pid" && kill -9 "$pid" 2>/dev/null
  done
  [[ -n "$SSHD_PID" ]] && { kill -CONT "$SSHD_PID"; kill "$SSHD_PID"; } 2>/dev/null
  [[ -n "$UV_PID" ]] && kill "$UV_PID" 2>/dev/null
  for name in "$M" "$D" "$R"; do
    case "$name" in t[0-9]*-*) rm -rf "${LINK_ROOT:?}/$name" ;; esac
  done
}

link_log() { printf '%s\n' "$LINK_ROOT/$1/link.log"; }
gate_class() {
  python3 -I -c 'import json,sys
try: print(json.load(open(sys.argv[1]))["class"])
except FileNotFoundError: print("open")' "$LINK_ROOT/$1/gate.json"
}
open_gate() { rm -f "$LINK_ROOT/$1/gate.json"; }
ssh_starts() { grep -c " SSH pid=.* machine=$1 " "$SSH_LOG" 2>/dev/null || true; }
starts() { grep -c " START " "$(link_log "$1")" 2>/dev/null || true; }
# The time of the n-th (1-based) line of $1's link log matching $2.
log_time() { grep " $2" "$(link_log "$1")" | sed -n "${3:-1}p" | awk '{print $1}'; }
last_pid() { grep " $2" "$(link_log "$1")" | tail -n 1 | sed 's/.* pid=\([0-9]*\) .*/\1/'; }
# Seconds from the START of the dial whose watcher last wrote auth_stalled
# to that write (the START is the last EXEC's before the write).
stall_secs() {
  python3 -I - "$(link_log "$1")" <<'PY'
import sys
rows = [line.split(" ", 3) for line in open(sys.argv[1]) if line.count(" ") >= 2]
gate = max(i for i, r in enumerate(rows) if r[2] == "GATE" and r[3].startswith("auth_stalled"))
execd = max(i for i, r in enumerate(rows[:gate]) if r[2] == "EXEC")
start = next(r for r in rows if r[1] == rows[execd][1] and r[2] == "START")
print(f"{float(rows[gate][0]) - float(start[0]):.2f}")
PY
}
# The watcher of the dial whose ssh was pid $2: the first WATCHER after its EXEC.
watcher_of() {
  python3 -I - "$(link_log "$1")" "$2" <<'PY'
import sys
rows = [line.split(" ", 3) for line in open(sys.argv[1]) if line.count(" ") >= 2]
execd = next(i for i, r in enumerate(rows) if r[1] == "pid=" + sys.argv[2] and r[2] == "EXEC")
print(next(r[1][4:] for r in rows[execd:] if r[2].startswith("WATCHER")))
PY
}
machine_state() {
  hc_scratch machine ls --json | python3 -I -c 'import json,sys
name = sys.argv[1]
print(next((m["state"] for m in json.load(sys.stdin)["machines"] if m["name"] == name), "absent"))' "$1"
}
# machine.connect on the local scratch core; prints "<seconds> <state>".
connect_now() {
  python3 -I - "$LOCAL_SOCK" "$1" <<'PY'
import json, socket, sys, time
s = socket.socket(socket.AF_UNIX); s.settimeout(40); s.connect(sys.argv[1])
started = time.time()
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": "machine.connect", "params": {"machine": sys.argv[2]}}) + "\n").encode())
reply = json.loads(s.makefile("rb").readline())
state = reply.get("result", {}).get("state") or reply.get("error", {}).get("data", {}).get("kind", "error")
print(f"{time.time() - started:.2f} {state}")
PY
}
# A terminal for the commands a person runs (add, login), with run_herdr's
# isolation: no inherited herdr or core socket, scratch XDG folders.
herdr_tty() {
  script -q /dev/null env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" SHELL=/bin/sh \
    "$@" </dev/null
}
# Like wait_until, for up to $2 seconds.
wait_for() {
  local what="$1" limit="$2"
  shift 2
  for _ in $(seq 1 $((limit * 10))); do
    if "$@"; then return 0; fi
    sleep 0.1
  done
  fail "timed out after ${limit}s waiting for: $what"
}
now() { python3 -I -c 'import time; print(time.time())'; }
since() { python3 -I -c 'import sys,time; print(f"{time.time() - float(sys.argv[1]):.2f}")' "$1"; }
gate_is() { [[ "$(gate_class "$1")" == "$2" ]]; }
state_is() { [[ "$(machine_state "$1")" == "$2" ]]; }
le() { python3 -I -c 'import sys; sys.exit(0 if float(sys.argv[1]) <= float(sys.argv[2]) else 1)' "$1" "$2"; }

# --- the fake ssh: one process owning the link, as OpenSSH is (probe fakessh.py)
mkdir -p "$BASE/bin" "$BASE/elsewhere"
echo relay > "$SSH_MODE"
cat > "$BASE/bin/ssh" <<FAKE
#!$PY -I
import os, pwd, signal, subprocess, sys, threading, time
remote = sys.argv[-1]
target = sys.argv[-2] if len(sys.argv) > 2 else ""
mode = open("$SSH_MODE").read().strip()
names = " ".join(sorted(os.environ))
agent = os.environ.get("SSH_AUTH_SOCK")
agent = "none" if agent is None else ("recorded" if agent == "$AGENT_SOCK" else "other")
home = "passwd" if os.environ.get("HOME") == pwd.getpwuid(os.getuid()).pw_dir else "other"
with open("$SSH_LOG", "a") as log:
    log.write(f"{time.time():.3f} SSH pid={os.getpid()} machine={target} mode={mode} home={home} agent={agent} env={names.replace(' ', ',')} \n")
if mode == "hang":
    time.sleep(3600); sys.exit(255)
if mode == "banner":
    sys.stderr.write("# Tailscale SSH requires an additional check.\n# To authenticate, visit: https://login.tailscale.com/a/0000\n")
    sys.stderr.flush(); time.sleep(3600); sys.exit(255)
if mode == "denied":
    sys.stderr.write(f"{target}: Permission denied (publickey).\n"); sys.exit(255)
if mode == "slow":
    time.sleep(5)
env = {"HOME": os.environ["HOME"], "USER": os.environ.get("USER", ""), "PATH": "/usr/bin:/bin:/usr/sbin:/sbin:" + os.environ["HOME"] + "/.local/bin",
       "XDG_RUNTIME_DIR": "$FAR_RUNTIME_DIR", "XDG_STATE_HOME": "$FAR_STATE_DIR"}
p = subprocess.Popen(["sh", "-c", remote], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, bufsize=0, start_new_session=True)
def die(*_):
    try: os.killpg(p.pid, signal.SIGKILL)
    except ProcessLookupError: pass
    os._exit(255)
signal.signal(signal.SIGTERM, die)
bytes_log = open("$BASE/bytes.log", "a", buffering=1)
def pump(src, dst, way):
    while True:
        try: b = os.read(src, 65536)
        except OSError: break
        if not b: break
        bytes_log.write(f"{time.time():.3f} {target} {way} {len(b)}\n")
        os.write(dst, b)
threading.Thread(target=pump, args=(0, p.stdin.fileno(), "up"), daemon=True).start()
out = threading.Thread(target=pump, args=(p.stdout.fileno(), 1, "down"), daemon=True); out.start()
rc = p.wait(); out.join(1); sys.exit(rc)
FAKE
chmod +x "$BASE/bin/ssh"

# V26's relay machine reaches the far core without a guard and would start
# it; this part needs a far core only herdr's link reaches.
hc_scratch machine rm loop >/dev/null || fail "could not remove V26's machine"
# The far core V26's link started may have ended with that link; this part
# starts it as a person would (core-phase3 D7), and the guard checks it.
hc_far server start >/dev/null 2>&1 || fail "could not start the far core"
wait_for "the far core's socket" 10 test -S "$FAR_SOCK"

# --- add without a terminal: refused, nothing added
if run_herdr machine add --core fake-target --name "$M" --remote-session "$FAR_SESSION" >"$BASE/add.out" 2>&1 </dev/null; then
  fail "machine add --core ran without a terminal"
fi
grep -q "A person runs it: herdr machine add --core fake-target --name $M" "$BASE/add.out" \
  || fail "the refusal does not name the command: $(cat "$BASE/add.out")"
[[ ! -e "$LINK_ROOT/$M" && "$(machine_state "$M")" == absent ]] || fail "a refused add left state behind"
echo "ok: machine add --core without a terminal refuses and names the command"

# --- add from a terminal (S1): the manual lease comes before hc's first dial (V33)
herdr_tty SSH_AUTH_SOCK="$AGENT_SOCK" PATH="$BASE/bin:$PATH" \
  "$HERDR_BIN" --session "$SESSION" machine add --core fake-target --name "$M" --remote-session "$FAR_SESSION" \
  >"$BASE/add.out" 2>&1 || fail "machine add --core failed: $(tail -n 5 "$BASE/add.out")"
wait_for "$M connected" 20 state_is "$M" connected
hc_scratch machine ls --json | python3 -I -c 'import json,sys
m = next(m for m in json.load(sys.stdin)["machines"] if m["name"] == sys.argv[1])
assert m["via"] and m["target"] == sys.argv[2] + " core-link " + sys.argv[1], m["target"]' "$M" "$HERDR_BIN" \
  || fail "core's target for $M is not herdr's link command"
LEASE_MTIME="$(python3 -I -c 'import os,sys; print(os.stat(sys.argv[1]).st_mtime)' "$LINK_ROOT/$M/leases/manual")"
le "$LEASE_MTIME" "$(log_time "$M" START)" || fail "the manual lease came after the first dial"
# Exec shape (F2): every dial's ssh is the core-link process itself.
for pid in $(grep " SSH pid=.* machine=fake-target " "$SSH_LOG" | sed 's/.* pid=\([0-9]*\) .*/\1/'); do
  grep -q " pid=$pid START " "$(link_log "$M")" || fail "ssh pid $pid is not a core-link pid: ssh was not exec'd"
done
# V35's pin, live: core's line is still `sh -c '...connect'`.
grep " START " "$(link_log "$M")" | head -n 1 | grep -qF "line=\"sh -c '\"..\"\$p\\\" connect'\"" \
  || fail "core's dial line changed shape: $(grep ' START ' "$(link_log "$M")" | head -n 1)"
echo "ok: $M added through core-link; lease before the first START; ssh is the via pid; line shape pinned"

# --- V31 (core-link part): ssh's environment is the allowlist plus the recorded agent
grep " SSH pid=.* machine=fake-target " "$SSH_LOG" | while read -r line; do
  names="$(sed 's/.* env=\([^ ]*\).*/\1/' <<<"$line")"
  for name in ${names//,/ }; do
    case "$name" in HOME|USER|LOGNAME|PATH|LANG|LC_*|TMPDIR|SSH_AUTH_SOCK|__CF_USER_TEXT_ENCODING) ;;
      *) fail "ssh got $name, which is not allowlisted" ;; esac
  done
  grep -q "home=passwd agent=recorded" <<<"$line" || fail "ssh did not get the passwd home and the recorded agent"
done
echo "ok: V31 ssh env is allowlisted names only, with the recorded SSH_AUTH_SOCK"

# --- a machine whose state exists but which core does not dial: core-link by
# hand. Core keeps one machine per far core, so its state is written here as
# machine add --core writes it, without adding it to core.
mkdir -p "$LINK_ROOT/$D/leases" && chmod 700 "$LINK_ROOT/$D"
printf '{"target":"direct-target","session":"%s","ssh":"%s","ssh_auth_sock":"%s"}\n' \
  "$FAR_SESSION" "$BASE/bin/ssh" "$AGENT_SOCK" > "$LINK_ROOT/$D/target.json"
link() {
  env -i HOME="$BASE/elsewhere" XDG_STATE_HOME="$BASE/elsewhere/state" SSH_AUTH_SOCK=/nonexistent/other \
    PATH="/usr/bin:/bin" HERDR_PANE_ID=w9:p9 HC_SESSION=elsewhere LANG=C \
    "$HERDR_BIN" core-link "$D" "$1"
}
ok_line="sh -c 'exit 0'"
before="$(ssh_starts direct-target)"
link "$ok_line" 2>"$BASE/link.err" || fail "a dial through core-link failed: $(cat "$BASE/link.err")"
[[ "$(ssh_starts direct-target)" == $((before + 1)) ]] || fail "core-link did not run ssh once"
tail -n 1 "$SSH_LOG" | grep -q "home=passwd agent=recorded" \
  || fail "with HOME, XDG_STATE_HOME and SSH_AUTH_SOCK elsewhere, ssh did not get the passwd home and recorded agent"
tail -n 1 "$SSH_LOG" | grep -q "HERDR_PANE_ID\|HC_SESSION\|XDG_STATE_HOME" && fail "ssh got a variable from core's environment"
[[ -z "$(ls -A "$BASE/elsewhere")" ]] || fail "core-link wrote under the HOME it was given"
grep -q " UP after " "$(link_log "$D")" || fail "the guard did not report the far core"
echo "ok: V31 core-link reads the passwd-home state whatever HOME, XDG_STATE_HOME and SSH_AUTH_SOCK say"

# --- V28: one dial at a time, and core's Windows line is never dialled
echo slow > "$SSH_MODE"
locked_before="$(grep -c " LOCKED " "$(link_log "$D")" || true)"
locked() { [[ "$(grep -c " LOCKED " "$(link_log "$D")")" -gt "$1" ]]; }
link "sh -c 'sleep 1'" 2>/dev/null &
first=$!
wait_for "the first dial holds dial.lock" 5 locked "$locked_before"
wait_for "the first dial's ssh" 5 sh -c "tail -n 1 '$SSH_LOG' | grep -q mode=slow"
before="$(ssh_starts direct-target)"
started="$(now)"
code=0; link "$ok_line" 2>/dev/null || code=$?
elapsed="$(since "$started")"
[[ "$code" == 75 ]] || fail "a second dial during the first exited $code, not 75"
le "$elapsed" 2.1 || fail "the second dial took ${elapsed}s to refuse"
grep -q "REFUSE busy" "$(link_log "$D")" || fail "the refusal was not busy"
code=0; link 'cmd /d /c "if exist .\AppData\herdr-core.exe (.\AppData\herdr-core.exe connect)"' 2>/dev/null || code=$?
[[ "$code" == 75 ]] && grep -q "REFUSE NotPosixLine" "$(link_log "$D")" || fail "core's Windows line was not refused ($code)"
alive_ssh=0
for _ in $(seq 1 30); do
  n=0
  for pid in $(grep " machine=direct-target " "$SSH_LOG" | sed 's/.* pid=\([0-9]*\) .*/\1/'); do alive "$pid" && n=$((n + 1)); done
  (( n > alive_ssh )) && alive_ssh=$n
  sleep 0.1
done
wait "$first" || true
[[ "$(ssh_starts direct-target)" == "$before" ]] || fail "a refused dial started ssh"
(( alive_ssh <= 1 )) || fail "$alive_ssh ssh processes ran at once"
echo "ok: V28 the second dial refused busy in ${elapsed}s, the Windows line refused, at most one ssh"

# --- V8: the failure classes close the gate until machine login
refused_after() {
  local before code=0
  before="$(ssh_starts direct-target)"
  link "$ok_line" 2>/dev/null || code=$?
  [[ "$code" == 75 && "$(ssh_starts direct-target)" == "$before" ]] || fail "after $1 a dial was not refused without ssh ($code)"
}
echo banner > "$SSH_MODE"
link "$ok_line" 2>/dev/null || true
[[ "$(gate_class "$D")" == needs_check ]] || fail "a Tailscale check banner gave gate $(gate_class "$D")"
refused_after needs_check
open_gate "$D"
echo denied > "$SSH_MODE"
link "$ok_line" 2>/dev/null || true
[[ "$(gate_class "$D")" == auth_failed ]] || fail "Permission denied gave gate $(gate_class "$D")"
refused_after auth_failed
open_gate "$D"
echo hang > "$SSH_MODE"
link "$ok_line" 2>/dev/null || true
[[ "$(gate_class "$D")" == auth_stalled ]] || fail "a silent ssh gave gate $(gate_class "$D")"
stall="$(stall_secs "$D")"
le "$stall" 15.5 || fail "auth_stalled was written ${stall}s after the START"
refused_after auth_stalled
echo relay > "$SSH_MODE"
herdr_tty PATH="$BASE/bin:$PATH" "$HERDR_BIN" --session "$SESSION" machine login "$D" >"$BASE/login.out" 2>&1 || true
[[ "$(gate_class "$D")" == open ]] || fail "machine login did not reopen the gate: $(cat "$BASE/login.out")"
before="$(ssh_starts direct-target)"
link "$ok_line" 2>/dev/null || fail "after machine login the dial failed"
[[ "$(ssh_starts direct-target)" == $((before + 1)) ]] || fail "after machine login no ssh ran"
echo "ok: V8 needs_check, auth_failed and auth_stalled (${stall}s) close the gate; machine login reopens it"

# --- V27: through core, a silent dial is classified before core's kill, and leaves nothing
echo hang > "$SSH_MODE"
link_ssh="$(last_pid "$M" START)"
ours_alive "$link_ssh" || fail "the link's ssh $link_ssh is not running"
kill "$link_ssh"
wait_for "core's redial stalls" 25 gate_is "$M" auth_stalled
sleep 1.5
for pid in $(grep -E " (START|WATCHER)" "$(link_log "$M")" | sed 's/.* pid=\([0-9]*\) .*/\1/' | sort -u); do
  ours_alive "$pid" && fail "pid $pid of a stalled dial is still alive"
done
open_gate "$M"
read -r took state < <(connect_now "$M")
le "$took" 17 || fail "machine.connect on a silent dial took ${took}s"
[[ "$(gate_class "$M")" == auth_stalled ]] || fail "the silent dial did not close the gate"
stall="$(stall_secs "$M")"
le "$stall" 15.5 || fail "through core, auth_stalled came ${stall}s after the START"
echo "ok: V27 machine.connect answered $state in ${took}s; gate after ${stall}s; no pid of the dial left"
# The same with the via pid SIGKILLed 3 s after its START, as core does.
open_gate "$M"
n="$(starts "$M")"
connect_now "$M" >/dev/null &
connecting=$!
more_starts() { [[ "$(starts "$1")" -gt "$2" ]]; }
wait_for "a new dial" 10 more_starts "$M" "$n"
wait_for "the new dial's ssh" 5 sh -c "tail -n 1 '$SSH_LOG' | grep -q mode=hang"
via="$(last_pid "$M" EXEC)"
sleep 3
kill -9 "$via"
sleep 1.5
watcher="$(watcher_of "$M" "$via")"
ours_alive "$via" && fail "the killed via $via is alive"
ours_alive "$watcher" && fail "the watcher $watcher outlived its ssh"
wait "$connecting" || true
echo "ok: V27 kill -9 of the via (ssh $via) left no watcher ($watcher)"

# --- V35: a dial never starts the far core
wait_for "core's retries settle on the closed gate" 25 gate_is "$M" auth_stalled
echo relay > "$SSH_MODE"
hc_far server stop >/dev/null || fail "could not stop the far core"
wait_until "far socket gone" test ! -S "$FAR_SOCK"
open_gate "$M"
read -r took state < <(connect_now "$M")
[[ "$(gate_class "$M")" == far_core_down ]] || fail "a stopped far core gave gate $(gate_class "$M")"
[[ ! -e "$FAR_SOCK" ]] || fail "the dial started the far core"
before="$(ssh_starts fake-target)"
read -r took state < <(connect_now "$M")
[[ "$(ssh_starts fake-target)" == "$before" ]] || fail "a dial ran while far_core_down held the gate"
grep -q "REFUSE Gate(FarCoreDown)" "$(link_log "$M")" || fail "far_core_down did not refuse the next dial"
# A stale socket, as a SIGKILLed core leaves.
hc_far server start >/dev/null 2>&1 || fail "could not start the far core"
far_core="$(hc_far server status --json | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["pid"])')"
kill -9 "$far_core"
wait_until "the far core gone" sh -c "! kill -0 $far_core 2>/dev/null"
[[ -S "$FAR_SOCK" ]] || fail "the killed far core left no socket file to test"
stale_inode="$(stat -f %i "$FAR_SOCK")"
open_gate "$M"
connect_now "$M" >/dev/null
[[ "$(gate_class "$M")" == far_core_down ]] || fail "a stale far socket gave gate $(gate_class "$M")"
[[ "$(stat -f %i "$FAR_SOCK")" == "$stale_inode" ]] && ! python3 -I -c 'import socket,sys; socket.socket(socket.AF_UNIX).connect(sys.argv[1])' "$FAR_SOCK" 2>/dev/null \
  || fail "the dial started a far core over the stale socket"
echo "ok: V35 stopped and stale far cores give far_core_down, no far core starts, the next dial is refused"
# A person starts the far core and logs in: the gate opens and core connects.
hc_far server start >/dev/null 2>&1 || fail "could not start the far core again"
lease_before="$(now)"
herdr_tty PATH="$BASE/bin:$PATH" "$HERDR_BIN" --session "$SESSION" machine login "$M" >"$BASE/login.out" 2>&1 \
  || fail "machine login $M failed: $(tail -n 3 "$BASE/login.out")"
grep -q "$M: connected" "$BASE/login.out" || fail "machine login did not connect: $(cat "$BASE/login.out")"
LEASE_MTIME="$(python3 -I -c 'import os,sys; print(os.stat(sys.argv[1]).st_mtime)' "$LINK_ROOT/$M/leases/manual")"
le "$lease_before" "$LEASE_MTIME" && le "$LEASE_MTIME" "$(log_time "$M" START "$(starts "$M")")" \
  || fail "login's lease did not come before its dial"
run_herdr machine status --core "$M" --json | python3 -I -c 'import json,sys
row = json.load(sys.stdin)[0]
assert row["state"] == "connected" and row["gate"] == "open" and row["herdr_link"], row
assert any(lease["holder"] == "manual" and lease["in_force"] for lease in row["leases"]), row' \
  || fail "machine status --core did not report the connected, open machine"
echo "ok: V33 machine login wrote its lease before the dial, opened the gate and connected; status reports it"

# Core keeps one machine per far core: $M goes before $R reaches the same one.
run_herdr machine rm --core "$M" >/dev/null 2>&1 || fail "machine rm --core $M failed"
[[ ! -e "$LINK_ROOT/$M" && "$(machine_state "$M")" == absent ]] || fail "machine rm --core left $M"
echo "ok: machine rm --core removed $M from core and its link state"

# --- V34 and V36: real OpenSSH to a loopback-only server
mkdir -p "$BASE/sshd"
ssh-keygen -q -t ed25519 -N '' -f "$BASE/sshd/ckey" -C herdr-test >/dev/null
ssh-keygen -q -t ed25519 -N '' -f "$BASE/sshd/hkey" -C herdr-test-host >/dev/null
PORT="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
printf '[127.0.0.1]:%s %s\n' "$PORT" "$(cut -d' ' -f1,2 "$BASE/sshd/hkey.pub")" > "$BASE/sshd/known_hosts"
cat > "$BASE/sshd/config" <<CFG
Host loopback
  HostName 127.0.0.1
  Port $PORT
  User $(id -un)
  IdentityFile $BASE/sshd/ckey
  IdentitiesOnly yes
  IdentityAgent none
  UserKnownHostsFile $BASE/sshd/known_hosts
  GlobalKnownHostsFile /dev/null
CFG
cat > "$BASE/sshd/server.py" <<SERVER
import os, socket, subprocess, sys, threading, time
import paramiko
LOG = open("$BASE/sshd/auth.log", "a", buffering=1)
client_key = paramiko.Ed25519Key.from_private_key_file("$BASE/sshd/ckey")
host_key = paramiko.Ed25519Key.from_private_key_file("$BASE/sshd/hkey")
ENV = {"HOME": os.environ["HOME"], "USER": os.environ.get("USER", ""), "PATH": "/usr/bin:/bin:" + os.environ["HOME"] + "/.local/bin",
       "XDG_RUNTIME_DIR": "$FAR_RUNTIME_DIR", "XDG_STATE_HOME": "$FAR_STATE_DIR"}
class Server(paramiko.ServerInterface):
    def check_auth_publickey(self, username, key):
        ok = key.asbytes() == client_key.asbytes()
        if ok: LOG.write(f"{time.time():.3f} AUTH ok\n")
        return paramiko.AUTH_SUCCESSFUL if ok else paramiko.AUTH_FAILED
    def get_allowed_auths(self, username): return "publickey"
    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
    def check_channel_exec_request(self, channel, command):
        LOG.write(f"{time.time():.3f} EXEC\n")
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
    t = paramiko.Transport(sock); t.add_server_key(host_key); t.start_server(server=Server())
    while t.is_active(): time.sleep(0.5)
ls = socket.socket(); ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); ls.bind(("127.0.0.1", $PORT)); ls.listen(8)
LOG.write(f"{time.time():.3f} LISTEN\n")
while True:
    s, _ = ls.accept(); threading.Thread(target=handle, args=(s,), daemon=True).start()
SERVER
(cd "$BASE/sshd" && exec uv run --no-project --quiet --with paramiko python -I "$BASE/sshd/server.py") >"$BASE/sshd/server.out" 2>&1 &
UV_PID=$!
wait_for "the loopback server listens" 60 grep -qs LISTEN "$BASE/sshd/auth.log"
# The python process holding the port, not uv.
SSHD_PID="$(lsof -nP -t -iTCP:"$PORT" -sTCP:LISTEN | head -n 1)"
[[ "$SSHD_PID" =~ ^[0-9]+$ ]] || fail "no loopback server pid"
auths() { grep -c "AUTH ok" "$BASE/sshd/auth.log" || true; }

herdr_tty SSH_AUTH_SOCK= PATH="/usr/bin:/bin:/usr/sbin:/sbin" "$HERDR_BIN" --session "$SESSION" \
  machine add --core loopback --name "$R" --remote-session "$FAR_SESSION" --ssh-config "$BASE/sshd/config" \
  >"$BASE/add.out" 2>&1 || fail "machine add --core over OpenSSH failed: $(tail -n 5 "$BASE/add.out")"
wait_for "$R connected" 20 state_is "$R" connected
ssh_pid="$(last_pid "$R" START)"
[[ "$(ps -o comm= -p "$ssh_pid")" == */ssh || "$(ps -o comm= -p "$ssh_pid")" == ssh ]] || fail "the via pid $ssh_pid is not ssh"
ps -o command= -p "$ssh_pid" | grep -q "ControlMaster=no -o ControlPath=none -o ServerAliveInterval=5 -o ServerAliveCountMax=2" \
  || fail "the link ssh lacks D10's options"
if lsof -nP -a -p "$ssh_pid" -U -F n 2>/dev/null | grep -q '^n/'; then fail "the link ssh holds a named unix socket (a control master)"; fi
before="$(auths)"
kill "$ssh_pid"
more_auths() { [[ "$(auths)" -gt "$1" ]]; }
wait_for "core redials $R" 20 more_auths "$before"
wait_for "$R connected again" 20 state_is "$R" connected
ours_alive "$ssh_pid" && fail "the ended ssh $ssh_pid is alive"
echo "ok: V34 no control master or socket; after the link ended the redial authenticated again ($before -> $(auths))"

kill -STOP "$SSHD_PID"
froze="$(now)"
unreachable_after=""
for _ in $(seq 1 20); do
  t0="$(now)"
  hc_scratch ls --json >/dev/null 2>&1
  latency="$(since "$t0")"
  state="$(machine_state "$R")"
  at="$(since "$froze")"
  echo "  V36 t+${at}s terminal.list ${latency}s machine $state"
  if [[ "$state" == unreachable ]]; then unreachable_after="$at"; break; fi
  sleep 2
done
kill -CONT "$SSHD_PID"
[[ -n "$unreachable_after" ]] && le "$unreachable_after" 30 || fail "a frozen link was not unreachable within 30 s"
echo "ok: V36 a frozen loopback server made $R unreachable after ${unreachable_after}s"

# --- the end: remove the machines; nothing of this part keeps running
run_herdr machine rm --core "$R" >/dev/null 2>&1 || fail "machine rm --core $R failed"
[[ ! -e "$LINK_ROOT/$R" && "$(machine_state "$R")" == absent ]] || fail "machine rm --core left $R"
kill "$SSHD_PID" 2>/dev/null
wait "$UV_PID" 2>/dev/null || true
none_left() {
  local pid
  for pid in $(logged_pids); do ours_alive "$pid" && return 1; done
  ! alive "$SSHD_PID"
}
wait_for "every dial's processes to end" 10 none_left
echo "ok: machines removed; no ssh, watcher, core-link or loopback server left"
