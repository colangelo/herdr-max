# Sourced by scripts/core_machine_live_test.sh after its other parts, with
# its scratch cores, helpers and trap (openspec/changes/core-phase5, PR E:
# the fleet CLI over the link).
#
# `herdr --machine <core machine>` through `machine.exec` (D7): V18 (the
# same JSON as the old remote's `--machine` to the same far herdr), V31's
# exec part (the far herdr runs without the far core's HERDR_* variables),
# E.1 (the far herdr is probed once per link), V32 (a dormant machine
# refuses with exit 75 and writes nothing; `--dial` writes its lease before
# the one dial, and removes it after), and `--dial` refused over a gate only
# a login reopens.
#
# The far side is a scratch core on this Mac with its own herdr server,
# reached through herdr's own link command and a fake `ssh` that runs the
# far line against it; the same fake ssh stands in for the old remote's
# OpenSSH to that herdr. Nothing dials another host. The machine's state
# folder lives under the passwd home with a name unique to this run and is
# removed at the end.

[[ -n "${BASE:-}" && -n "${CORE_SESSION:-}" ]] || { echo "source this from core_machine_live_test.sh" >&2; exit 1; }

FL_HOME="$(python3 -I -c 'import os, pwd; print(pwd.getpwuid(os.getuid()).pw_dir)')"
FL_ROOT="$FL_HOME/.local/state/herdr-dev/core-machines"
FT="f$$-fleet"
FL_DIR="$FL_ROOT/$FT"
# The far herdr session and the far core session that is its primary, as
# a debug build names it: `--machine` finds the one from the other.
FL_SESSION="fl$$"
FL_CORE="herdrdev-$FL_SESSION"
F3R="$BASE/f3r"
F3S="$BASE/f3s"
F3C="$BASE/f3c"
F3B="$BASE/f3bin"
F3_SOCK="$F3R/herdr-core/$FL_CORE.sock"
FL_BIN="$BASE/flbin"
FL_SSH_LOG="$BASE/fl-ssh.log"
FL_FAR_LOG="$BASE/fl-far.log"
FL_LOCAL_SOCK="$RUNTIME_DIR/herdr-core/$CORE_SESSION.sock"
FL_FAR_API="$F3C/herdr-dev/sessions/$FL_SESSION/herdr.sock"
FL_HC="$(command -v hc)"
FL_PY="$(command -v python3)"
FL_PROFILE="0123456789abcdef0123456789abcd$(printf '%02x' $(( $$ % 256 )))"
FL_ENDPOINTS="$STATE_DIR/herdr-dev/client/endpoints.json"

hc_fl() {
  env -i HOME="$HOME" USER="${USER:-}" PATH="$F3B:/usr/bin:/bin" TMPDIR="${TMPDIR:-/tmp}" \
    XDG_RUNTIME_DIR="$F3R" XDG_STATE_HOME="$F3S" XDG_CONFIG_HOME="$F3C" HC_SESSION="$FL_CORE" \
    "$FL_HC" "$@"
}

far_fl_herdr() {
  env -i HOME="$HOME" USER="${USER:-}" PATH="/usr/bin:/bin" TMPDIR="${TMPDIR:-/tmp}" SHELL=/bin/sh \
    XDG_RUNTIME_DIR="$F3R" XDG_STATE_HOME="$F3S" XDG_CONFIG_HOME="$F3C" \
    "$HERDR_BIN" --session "$FL_SESSION" "$@"
}

# The fleet CLI here: the local scratch session by HERDR_SESSION (`--machine`
# takes no `--session`), the fake ssh first on PATH.
fleet_cli() {
  env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_WORKSPACE_ID \
    -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION -u CLAUDECODE -u HERDR_MACHINE_DIAL \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HERDR_SESSION="$SESSION" SHELL=/bin/sh PATH="$FL_BIN:$PATH" "$HERDR_BIN" "$@"
}

fl_tty() {
  script -q /dev/null env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" SHELL=/bin/sh \
    PATH="$FL_BIN:$PATH" "$@" </dev/null
}

eval "$(declare -f link_cleanup | sed '1s/link_cleanup/link_cleanup_before_fleet/')"
link_cleanup() {
  set +e
  [[ -S "$FL_FAR_API" ]] && far_fl_herdr server stop >/dev/null 2>&1
  [[ -S "$FL_LOCAL_SOCK" ]] && hc_scratch machine rm "$FT" >/dev/null 2>&1
  local pid
  for pid in $(awk '{for (i = 1; i <= NF; i++) if ($i ~ /^pid=[0-9]+$/) print substr($i, 5)}' \
      "$FL_SSH_LOG" "$FL_DIR/link.log" 2>/dev/null | sort -u); do
    ps -o command= -p "$pid" 2>/dev/null | grep -q -e "$BASE" -e "$FT" && kill -9 "$pid" 2>/dev/null
  done
  [[ -S "$F3_SOCK" ]] && hc_fl server stop >/dev/null 2>&1
  case "$FT" in f[0-9]*-fleet) rm -rf "${FL_ROOT:?}/$FT" ;; esac
  link_cleanup_before_fleet
}

fl_wait() {
  local what="$1" secs="$2"
  shift 2
  local until=$((SECONDS + secs))
  while ((SECONDS < until)); do
    if "$@"; then return 0; fi
    sleep 0.2
  done
  fail "timed out after ${secs}s waiting for: $what"
}
fl_state() {
  hc_scratch machine ls --json | python3 -I -c 'import json,sys
print(next((m["state"] for m in json.load(sys.stdin)["machines"] if m["name"] == sys.argv[1]), "absent"))' "$FT"
}
fl_is() { [[ "$(fl_state)" == "$1" ]]; }
fl_ssh_starts() { grep -c " SSH pid=.* machine=fleet-target " "$FL_SSH_LOG" 2>/dev/null || true; }
fl_cli_leases() { find "$FL_DIR/leases" -name 'cli-*' 2>/dev/null | wc -l | tr -d ' '; }
fl_log_count() { grep -c " $1" "$FL_DIR/link.log" 2>/dev/null || true; }
fl_more() { [[ "$(fl_log_count "$1")" -gt "$2" ]]; }
fl_probes() { grep -c "args=remote-api-bridge --check" "$FL_FAR_LOG" 2>/dev/null || true; }
# The same JSON, whitespace and key order aside.
same_json() {
  python3 -I -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])) == json.load(open(sys.argv[2])) else 1)' "$1" "$2"
}

echo "--- PR E: the fleet CLI over core machine $FT"
mkdir -p "$F3R" "$F3S" "$F3C/herdr-dev" "$F3B" "$FL_BIN"
printf 'onboarding = false\n' > "$F3C/herdr-dev/config.toml"
# The far herdr, behind a wrapper that logs each run's arguments and the
# names (never the values) of the HERDR_* variables it got.
cat > "$F3B/herdr" <<FAR
#!/bin/sh
printf '%s args=%s env=%s\n' "\$(date +%s)" "\$*" "\$(env | sed 's/=.*//' | grep '^HERDR_' | sort | tr '\n' ',')" >> "$FL_FAR_LOG"
exec "$HERDR_BIN" "\$@"
FAR
chmod +x "$F3B/herdr"

# The fake ssh: herdr's link to the far core (target fleet-target), or the
# old remote's ssh to the far herdr (target fake-mac, as tests/machine_api.rs
# fakes it).
cat > "$FL_BIN/ssh-old" <<'OLD'
#!/bin/sh
control=; previous=; operation=; configuration=
for arg do
    last=$arg
    if [ "$previous" = -S ]; then control=$(printf '%s' "$arg" | sed 's/%C/fixture/'); fi
    if [ "$previous" = -O ]; then operation=$arg; fi
    if [ "$arg" = -G ]; then configuration=yes; fi
    previous=$arg
done
if [ -n "$configuration" ]; then printf 'controlpath %s\n' "$control"; exit 0; fi
if [ "$operation" = stop ]; then exit 0; fi
if [ "$last" = exit ]; then exit 0; fi
case "$last" in
    '/bin/sh -c '*) exec /bin/sh -c "$last" ;;
    '/bin/sh -s')
        script=$(cat)
        case "$script" in
            *'uname -s'*) printf 'herdr-remote-output-ready:1\n'; uname -s; uname -m ;;
            *) exec /bin/sh -c "$script" ;;
        esac ;;
    *) echo "unexpected command: $last" >&2; exit 2 ;;
esac
OLD
chmod +x "$FL_BIN/ssh-old"
cat > "$FL_BIN/ssh" <<FAKE
#!$FL_PY -I
import os, signal, subprocess, sys, threading, time
args = sys.argv[1:]
far = {"HOME": os.environ["HOME"], "USER": os.environ.get("USER", ""), "PATH": "$F3B:/usr/bin:/bin",
       "TMPDIR": os.environ.get("TMPDIR", "/tmp"), "XDG_RUNTIME_DIR": "$F3R", "XDG_STATE_HOME": "$F3S",
       "XDG_CONFIG_HOME": "$F3C"}
if "fake-mac" in args:
    with open("$FL_SSH_LOG", "a") as log:
        log.write(f"{time.time():.3f} OLD pid={os.getpid()} last={args[-1][:40]!r}\n")
    os.execve("/bin/sh", ["/bin/sh", "$FL_BIN/ssh-old", *args], far)
remote, target = args[-1], args[-2]
with open("$FL_SSH_LOG", "a") as log:
    log.write(f"{time.time():.3f} SSH pid={os.getpid()} machine={target} \n")
p = subprocess.Popen(["sh", "-c", remote], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=far, bufsize=0, start_new_session=True)
def die(*_):
    try: os.killpg(p.pid, signal.SIGKILL)
    except ProcessLookupError: pass
    os._exit(255)
signal.signal(signal.SIGTERM, die)
def pump(src, dst):
    while True:
        try: b = os.read(src, 65536)
        except OSError: break
        if not b: break
        os.write(dst, b)
threading.Thread(target=pump, args=(0, p.stdin.fileno()), daemon=True).start()
out = threading.Thread(target=pump, args=(p.stdout.fileno(), 1), daemon=True); out.start()
rc = p.wait(); out.join(1); sys.exit(rc)
FAKE
chmod +x "$FL_BIN/ssh"

# The far core, started as a person would, with the six integration
# variables a bare `hc` in a herdr pane there would give it (F6): a far
# herdr that kept HERDR_SOCKET_PATH would talk to a socket that is not its
# server's.
env -i HOME="$HOME" USER="${USER:-}" PATH="$F3B:/usr/bin:/bin" TMPDIR="${TMPDIR:-/tmp}" \
  XDG_RUNTIME_DIR="$F3R" XDG_STATE_HOME="$F3S" XDG_CONFIG_HOME="$F3C" HC_SESSION="$FL_CORE" \
  HERDR_ENV=1 HERDR_PANE_ID=w9:p9 HERDR_TAB_ID=w9:t9 HERDR_WORKSPACE_ID=w9 \
  HERDR_SOCKET_PATH="$BASE/nowhere.sock" HERDR_CLIENT_SOCKET_PATH="$BASE/nowhere-client.sock" \
  "$FL_HC" server start >/dev/null 2>&1 || fail "could not start the far core $FL_CORE"
fl_wait "the far core's socket" 10 test -S "$F3_SOCK"
far_fl_herdr server >/dev/null 2>&1 &
fl_wait "the far herdr's socket" 20 test -S "$FL_FAR_API"
python3 -I - "$FL_FAR_API" "$BASE" <<'PY' || fail "could not create a far workspace"
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall((json.dumps({"id": "fl", "method": "workspace.create", "params": {"cwd": sys.argv[2], "label": "far-work"}}) + "\n").encode())
reply = json.loads(s.makefile("rb").readline())
assert "result" in reply, reply
PY

# Added by a person in a terminal (S1): its manual lease, then hc's dials.
fl_tty "$HERDR_BIN" --session "$SESSION" machine add --core fleet-target --name "$FT" --remote-session "$FL_CORE" \
  >"$BASE/fl-add.out" 2>&1 || fail "machine add --core $FT failed: $(tail -n 5 "$BASE/fl-add.out")"
fl_wait "$FT connected" 20 fl_is connected
grep -q "\"ssh\": \"$FL_BIN/ssh\"" "$FL_DIR/target.json" || fail "the machine was added with another ssh"

# --- V18 and E.1: the far side's JSON through the link, the far herdr probed once
code=0; fleet_cli --machine "$FT" pane list >"$BASE/fl-core.json" 2>"$BASE/fl-core.err" || code=$?
[[ "$code" == 0 ]] || fail "--machine $FT pane list exited $code: $(cat "$BASE/fl-core.err")"
python3 -I -c 'import json,sys; panes = json.load(open(sys.argv[1]))["result"]["panes"]; assert panes, "no panes"' \
  "$BASE/fl-core.json" || fail "the far pane list has no panes: $(head -c 300 "$BASE/fl-core.json")"
far_fl_herdr pane list >"$BASE/fl-local.json" 2>/dev/null || fail "the far herdr's own pane list failed"
same_json "$BASE/fl-core.json" "$BASE/fl-local.json" \
  || fail "--machine $FT gave other JSON than the far herdr itself: $(head -c 300 "$BASE/fl-core.json")"
[[ "$(fl_probes)" == 1 ]] || fail "the far herdr was probed $(fl_probes) times for one call"
fleet_cli --machine "$FT" pane list >"$BASE/fl-core2.json" 2>/dev/null || fail "the second call failed"
same_json "$BASE/fl-core.json" "$BASE/fl-core2.json" || fail "two calls gave different JSON"
[[ "$(fl_probes)" == 1 ]] || fail "the far herdr was probed again over the same link ($(fl_probes))"
grep -q "args=--session $FL_SESSION remote-api-bridge" "$FL_FAR_LOG" || fail "no request went to the far herdr session $FL_SESSION"
[[ "$(fl_cli_leases)" == 0 ]] || fail "a cli lease outlived its call"
fleet_cli machine status --core "$FT" --json | python3 -I -c 'import json,sys
row = json.load(sys.stdin)[0]
assert row["far_herdr"]["path"] == sys.argv[1], row["far_herdr"]' "$F3B/herdr" \
  || fail "machine status --core does not report the far herdr"
echo "ok: E.1 the far herdr ($F3B/herdr) was probed once for two calls over one link; status reports it"

# --- V31 (exec part): the far core has the six names, the far herdr none
hc_scratch exec --json --on "$FT" sh -c 'env | sed "s/=.*//" | grep "^HERDR_" | sort' >"$BASE/fl-control.json" \
  || fail "the control exec on $FT failed"
python3 -I -c 'import base64,json,sys
out = json.load(open(sys.argv[1]))
text = out.get("stdout", "")
try: text = base64.b64decode(text).decode()
except Exception: pass
assert "HERDR_SOCKET_PATH" in text and "HERDR_PANE_ID" in text, text' "$BASE/fl-control.json" \
  || fail "the far core lacks the fake variables, so the check proves nothing: $(head -c 300 "$BASE/fl-control.json")"
if grep "args=" "$FL_FAR_LOG" | grep -v "env=$"; then
  fail "a far herdr run through the link got HERDR_* variables"
fi
echo "ok: V31 the far core carries HERDR_* names; every far herdr run through --machine got none"

# --- V18: the old remote's --machine to the same far herdr gives the same JSON
if [[ -e "$FL_ENDPOINTS" ]]; then fail "an endpoints catalog already exists in the scratch state"; fi
mkdir -p "$(dirname "$FL_ENDPOINTS")"
printf '{"version":1,"ssh":[{"id":"%s","label":"fl-mac","target":"fake-mac","session":"%s","enabled":true}]}\n' \
  "$FL_PROFILE" "$FL_SESSION" > "$FL_ENDPOINTS"
code=0; fleet_cli --machine fl-mac pane list >"$BASE/fl-ssh.json" 2>"$BASE/fl-ssh.err" || code=$?
[[ "$code" == 0 ]] || fail "the old remote's --machine exited $code: $(tail -n 3 "$BASE/fl-ssh.err")"
grep -q " OLD pid=" "$FL_SSH_LOG" || fail "the old remote did not use the fake ssh"
same_json "$BASE/fl-core.json" "$BASE/fl-ssh.json" \
  || fail "the old remote and the link gave different JSON: $(head -c 200 "$BASE/fl-ssh.json")"
rm -f "$FL_ENDPOINTS"
echo "ok: V18 --machine through the link equals the old remote's --machine (and the far herdr's own) JSON"

# --- V32: a dormant machine refuses without --dial, and --dial makes one dial
ends="$(fl_log_count "LEASE END")"
python3 -I -c 'import os,sys,time
d = sys.argv[1]; tmp = os.path.join(d, "manual.new")
open(tmp, "w").write(f"{time.time() - 1:.3f} {time.time() - 60:.3f}\n"); os.replace(tmp, os.path.join(d, "manual"))' "$FL_DIR/leases"
fl_wait "$FT's link ends with its lease" 15 fl_more "LEASE END" "$ends"
fl_wait "$FT unreachable" 15 fl_is unreachable
starts="$(fl_ssh_starts)"
code=0; fleet_cli --machine "$FT" pane list >"$BASE/fl-dormant.out" 2>"$BASE/fl-dormant.err" || code=$?
[[ "$code" == 75 ]] || fail "--machine on a dormant machine exited $code, not 75: $(cat "$BASE/fl-dormant.err")"
grep -q "$FT is dormant; reaching it opens ssh (a Touch ID or Tailscale check can prompt). Announce it, then rerun with --dial, or run herdr machine wake $FT." "$BASE/fl-dormant.err" \
  || fail "the refusal does not say D7's message: $(cat "$BASE/fl-dormant.err")"
[[ ! -s "$BASE/fl-dormant.out" ]] || fail "the refusal printed output"
[[ "$(fl_cli_leases)" == 0 && "$(fl_ssh_starts)" == "$starts" ]] || fail "the refusal wrote a lease or started ssh"
echo "ok: V32 a dormant machine refuses with exit 75 and D7's message; no lease, no ssh"

dials="$(fl_log_count "CLI --dial")"
starts_log="$(fl_log_count "START")"
code=0; fleet_cli --machine "$FT" --dial pane list >"$BASE/fl-dial.json" 2>"$BASE/fl-dial.err" || code=$?
[[ "$code" == 0 ]] || fail "--dial exited $code: $(cat "$BASE/fl-dial.err")"
same_json "$BASE/fl-core.json" "$BASE/fl-dial.json" || fail "--dial gave other JSON"
[[ "$(fl_ssh_starts)" == $((starts + 1)) ]] || fail "--dial started $(( $(fl_ssh_starts) - starts )) ssh, not 1"
python3 -I - "$FL_DIR/link.log" "$dials" "$starts_log" <<'PY' || fail "the --dial lease did not come before the dial: $(tail -n 8 "$FL_DIR/link.log")"
import sys
rows = [line.split(" ", 3) for line in open(sys.argv[1]) if line.count(" ") >= 2]
dial = [r for r in rows if r[2] == "CLI"][int(sys.argv[2])]
start = [r for r in rows if r[2] == "START"][int(sys.argv[3])]
execd = [r for r in rows if r[2] == "EXEC" and r[1] == start[1]]
assert float(dial[0]) <= float(start[0]) and execd, (dial, start)
PY
[[ "$(fl_cli_leases)" == 0 ]] || fail "the --dial lease outlived the call"
echo "ok: V32 --dial wrote its lease before the one dial, gave the far JSON, and removed the lease"

# --- --dial over a gate only a login reopens: refused, no lease, no ssh
fl_wait "$FT dormant again" 20 fl_is unreachable
printf '{"class":"auth_failed","at":%s}\n' "$(python3 -I -c 'import time; print(time.time())')" > "$FL_DIR/gate.json"
starts="$(fl_ssh_starts)"
code=0; fleet_cli --machine "$FT" --dial pane list >/dev/null 2>"$BASE/fl-gate.err" || code=$?
[[ "$code" == 1 ]] && grep -q "herdr machine login $FT" "$BASE/fl-gate.err" \
  || fail "--dial over auth_failed gave $code: $(cat "$BASE/fl-gate.err")"
[[ "$(fl_cli_leases)" == 0 && "$(fl_ssh_starts)" == "$starts" ]] || fail "--dial over a closed gate wrote a lease or dialled"
rm -f "$FL_DIR/gate.json"
echo "ok: --dial over auth_failed refuses, names machine login, writes nothing and dials nothing"

far_fl_herdr server stop >/dev/null 2>&1
fleet_cli machine rm --core "$FT" >/dev/null 2>&1 || fail "machine rm --core $FT failed"
[[ ! -e "$FL_DIR" ]] || fail "machine rm --core left $FT's state"
hc_fl server stop >/dev/null 2>&1
echo "ok: fleet part done"
