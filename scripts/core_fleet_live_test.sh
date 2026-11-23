# Sourced by scripts/core_machine_live_test.sh after its other parts, with
# its scratch cores, helpers and trap (openspec/changes/core-phase5, PR E:
# the fleet CLI over the link).
#
# `herdr --machine <core machine>` through `machine.exec` (D7): V18 (the
# same JSON as the old remote's `--machine` to the same far herdr), V31's
# exec part (the far herdr runs without the far core's HERDR_* variables),
# E.1 (the far herdr is probed once per link), V32 (a dormant machine
# refuses with exit 75 and writes nothing; `--dial` writes its lease before
# the one dial, and removes it after), `--dial` refused over a gate only a
# login reopens, a SIGTERM that removes the lease and the bridge folder, and
# a link dropped during a call that is not redialled on its account; E.3
# and V20 live: a new tab named `$(touch x); w` from a view of the machine
# reaches the far herdr as that one name, through the same far bridge, and
# no shell ran it; and `prefix x` in the view on that tab's pane closes
# exactly that far pane, ending its program and no other (review M5), as
# the guarded `pane.close_if_terminal`; and a far pane whose program is
# respawned between the view's own check and the request is refused by the
# far herdr (`terminal_mismatch`) and stays (fork issue 316, M1).
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
# Each request line's method the far bridge got, each reply it gave, and a
# script the far wrapper runs once just before the next request (to change
# the far pane under a request already checked here).
FL_FAR_REQ_LOG="$BASE/fl-far-requests.log"
FL_FAR_REPLY_LOG="$BASE/fl-far-replies.log"
FL_FAR_HOOK="$BASE/fl-far-hook.sh"
FL_LOCAL_SOCK="$RUNTIME_DIR/herdr-core/$CORE_SESSION.sock"
FL_FAR_API="$F3C/herdr-dev/sessions/$FL_SESSION/herdr.sock"
FL_HC="$(command -v hc)"
FL_PY="$(command -v python3)"
FL_PROFILE="0123456789abcdef0123456789abcd$(printf '%02x' $(( $$ % 256 )))"
FL_ENDPOINTS="$STATE_DIR/herdr-dev/client/endpoints.json"
FL_TMUX="hcf$$"
FL_VIEW_LOCK="$CONFIG_HOME/herdr-dev/sessions/$SESSION/herdr-view-$FT.lock"
FL_VIEW_LOG="$CONFIG_HOME/herdr-dev/sessions/$SESSION/herdr-view-$FT.log"
fl_tm() { tmux -L "$FL_TMUX" -f /dev/null "$@"; }
# The view a client of the test started, by the pid in its lock, checked to
# be a herdr.
fl_stop_view() {
  local pid
  pid="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["pid"])' "$FL_VIEW_LOCK" 2>/dev/null)"
  [[ -n "$pid" ]] && ps -o comm= -p "$pid" 2>/dev/null | grep -q herdr && kill "$pid" 2>/dev/null
  return 0
}

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
    -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HERDR_SESSION="$SESSION" SHELL=/bin/sh PATH="$FL_BIN:$PATH" "$HERDR_BIN" "$@"
}
# The same in the background, as the herdr process itself ($! is its pid).
fleet_cli_bg() {
  ( exec env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_WORKSPACE_ID \
    -u HERDR_TAB_ID -u HERDR_PANE_ID -u HC_SOCKET -u HC_SESSION -u CLAUDECODE \
    XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
    HERDR_SESSION="$SESSION" SHELL=/bin/sh PATH="$FL_BIN:$PATH" "$HERDR_BIN" "$@" ) &
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
  fl_tm kill-server >/dev/null 2>&1
  fl_stop_view
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
fl_has_lease() { [[ -s "$FL_DIR/leases/cli-$1" ]]; }
fl_lease_is_keep() { [[ "$(awk '{print $3}' "$FL_DIR/leases/cli-$1" 2>/dev/null)" == keep ]]; }
fl_folders_of() { ls -d "${TMPDIR:-/tmp}"/herdr-fleet-"$1"-* /tmp/herdr-fleet-"$1"-* 2>/dev/null | wc -l | tr -d ' '; }
fl_log_count() { grep -c " $1" "$FL_DIR/link.log" 2>/dev/null || true; }
fl_more() { [[ "$(fl_log_count "$1")" -gt "$2" ]]; }
# Core retries fast right after a link ends (about 0.4 s, then doubling);
# a --dial connect that lands on one of them is restarted by core and
# counts as a failed dial (R4, D3a's accepted limit). Five refused retries
# after $1, the next is 8 s or more away.
fl_retries_backed_off() { fl_wait "core's retries to back off" 70 fl_more "REFUSE Dormant" $(( $1 + 4 )); }
fl_probes() { grep -c "args=remote-api-bridge --check" "$FL_FAR_LOG" 2>/dev/null || true; }
# The same JSON, whitespace and key order aside.
same_json() {
  python3 -I -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])) == json.load(open(sys.argv[2])) else 1)' "$1" "$2"
}

echo "--- PR E: the fleet CLI over core machine $FT"
mkdir -p "$F3R" "$F3S" "$F3C/herdr-dev" "$F3B" "$FL_BIN"
# Core runtime and the spaces document, so a view of the machine shows the
# far herdr's panes (E.3 below).
printf 'onboarding = false\n\n[experimental]\nruntime = "core"\ncore_spaces_doc = true\n' \
  > "$F3C/herdr-dev/config.toml"
# The far herdr, behind a wrapper that logs each run's arguments and the
# names (never the values) of the HERDR_* variables it got.
cat > "$F3B/herdr" <<FAR
#!/bin/sh
printf '%s args=%s env=%s\n' "\$(date +%s)" "\$*" "\$(env | sed 's/=.*//' | grep '^HERDR_' | sort | tr '\n' ',')" >> "$FL_FAR_LOG"
case "\$*" in
  *"remote-api-bridge --request")
    if [ -s "$FL_FAR_HOOK" ]; then /bin/sh "$FL_FAR_HOOK" >/dev/null 2>&1; : > "$FL_FAR_HOOK"; fi
    line="\$(head -n 1)"
    printf '%s\n' "\$line" | sed -n 's/^{"id":"[^"]*","method":"\([a-z_.]*\)".*/\1/p' >> "$FL_FAR_REQ_LOG"
    reply="\$(printf '%s\n' "\$line" | "$HERDR_BIN" "\$@")"; code=\$?
    printf '%s\n' "\$reply" >> "$FL_FAR_REPLY_LOG"
    printf '%s\n' "\$reply"
    exit \$code
    ;;
esac
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
FL_PANE="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["panes"][0]["pane_id"])' "$BASE/fl-core.json")"

# --- 2.1, 2.2: a call on a connected machine holds a keep lease of 60 s at
# most; SIGTERM removes it and the bridge folder at once
fleet_cli_bg --machine "$FT" pane wait-output "$FL_PANE" --match "never-printed-$$" --timeout 30000 \
  >/dev/null 2>"$BASE/fl-term.err"
term_pid=$!
fl_wait "the waiting call's lease" 10 fl_has_lease "$term_pid"
fl_lease_is_keep "$term_pid" || fail "a call on a connected machine wrote a lease that admits a dial: $(cat "$FL_DIR/leases/cli-$term_pid")"
python3 -I -c 'import sys,time; left = float(open(sys.argv[1]).read().split()[0]) - time.time(); assert left <= 61, left' \
  "$FL_DIR/leases/cli-$term_pid" || fail "the cli lease lasts more than a minute: $(cat "$FL_DIR/leases/cli-$term_pid")"
[[ "$(fl_folders_of "$term_pid")" == 1 ]] || fail "no bridge folder for the waiting call"
sleep 1
kill -TERM "$term_pid"
code=0; wait "$term_pid" || code=$?
[[ "$code" == 143 ]] || fail "the waiting call ended $code on SIGTERM, not 143"
[[ "$(fl_cli_leases)" == 0 ]] || fail "SIGTERM left the cli lease"
[[ "$(fl_folders_of "$term_pid")" == 0 ]] || fail "SIGTERM left the bridge folder"
echo "ok: 2.1 a connected call's lease keeps the link only and ends within 60 s; SIGTERM removed it and the folder (exit 143)"

# --- E.3 and V20 live: a new tab from a view of the machine, named
# `$(touch x); w`, typed into the view's own new-tab prompt by a person's
# keys. The view sends it as one JSON string through the far bridge; no
# shell there or here ran it, and the action's lease is gone after.
FL_NAME='$(touch x); w'
fl_x_files() {
  local f
  for f in "$HOME/x" "$PWD/x" "$BASE/x" "$F3C/x" "$F3S/x"; do [[ -e "$f" ]] && printf '%s ' "$f"; done
  return 0
}
x_before="$(fl_x_files)"
fl_tm new-session -d -s flview -x 170 -y 55 \
  "$(printf '%q ' env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
     -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID -u HERDR_PANE_ID -u TMUX -u HC_SOCKET -u HC_SESSION -u CLAUDECODE \
     XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" XDG_STATE_HOME="$STATE_DIR" \
     TERM=xterm-256color HERDR_NO_MACHINE_LEASES=1 SHELL=/bin/sh PATH="$FL_BIN:$PATH" \
     "$HERDR_BIN" --session "$SESSION" --remote "core:$FT") 2>$BASE/fl-view.err"
fl_tm set-option -t flview remain-on-exit on >/dev/null
fl_view_shows() { fl_tm capture-pane -p -t flview 2>/dev/null | grep -q "far-work"; }
fl_wait "the view of $FT showing the far space" 60 fl_view_shows
fl_tabs() {
  far_fl_herdr tab list 2>/dev/null \
    | python3 -I -c 'import json,sys; print("\n".join(t["label"] for t in json.load(sys.stdin)["result"]["tabs"]))'
}
tabs_before="$(fl_tabs | wc -l | tr -d ' ')"
bridges="$(grep -c "args=--session $FL_SESSION remote-api-bridge --request" "$FL_FAR_LOG" || true)"
fl_tm send-keys -t flview C-b c
sleep 1
fl_tm send-keys -t flview -l "$FL_NAME"
sleep 0.3
fl_tm send-keys -t flview Enter
fl_named_tab() { fl_tabs | grep -qxF "$FL_NAME"; }
fl_wait "the far tab named $FL_NAME" 30 fl_named_tab
[[ "$(fl_tabs | wc -l | tr -d ' ')" == $((tabs_before + 1)) ]] || fail "E.3: the view's new tab made $(( $(fl_tabs | wc -l) - tabs_before )) far tabs, not 1"
[[ "$(grep -c "args=--session $FL_SESSION remote-api-bridge --request" "$FL_FAR_LOG")" -gt "$bridges" ]] \
  || fail "E.3: the view's action did not go through the far bridge"
fl_view_follows() { grep 'view layout' "$FL_VIEW_LOG" 2>/dev/null | tail -n 1 | grep -q "tabs=$((tabs_before + 1))"; }
fl_wait "the view following the far tab" 10 fl_view_follows
[[ "$(fl_x_files)" == "$x_before" ]] \
  || fail "V20: a file x appeared: the name ran as a shell line"
[[ "$(fl_cli_leases)" == 0 ]] || fail "E.3: the view action's lease outlived it: $(ls "$FL_DIR/leases")"
echo "ok: E.3/V20 a new tab named '$FL_NAME' from a view of $FT is that far tab, through the far bridge; nothing ran it; no lease left"

# --- E.3 (review M5) live: `prefix x` on that new tab's pane in the view
# closes exactly that far pane over herdr's own link (D5: a person's close):
# its program ends; every other far pane keeps its terminal and program.
fl_far_rpc() {
  python3 -I - "$F3_SOCK" "$1" "$2" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile("rb")
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {}}) + "\n").encode())
f.readline()
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": sys.argv[2], "params": json.loads(sys.argv[3])}) + "\n").encode())
print(json.dumps(json.loads(f.readline()).get("result")))
PY
}
# The pid of the far core's terminal $1 while it is live, else nothing.
fl_term_pid() {
  fl_far_rpc terminal.get "{\"terminal\":\"$1\"}" 2>/dev/null \
    | python3 -I -c 'import json,sys; r = json.load(sys.stdin) or {}; print(r.get("pid", "") if r.get("state") == "live" else "")'
}
# Each far pane as "<pane id> <tab id> <core address>".
fl_far_panes() {
  far_fl_herdr pane list 2>/dev/null | python3 -I -c 'import json,sys
for p in json.load(sys.stdin)["result"]["panes"]: print(p["pane_id"], p["tab_id"], p.get("core_address") or "-")'
}
fl_new_tab="$(far_fl_herdr tab list 2>/dev/null | python3 -I -c 'import json,sys
print(next(t["tab_id"] for t in json.load(sys.stdin)["result"]["tabs"] if t["label"] == sys.argv[1]))' "$FL_NAME")"
read -r FL_CLOSE_PANE _ FL_CLOSE_ADDR < <(fl_far_panes | awk -v tab="$fl_new_tab" '$2 == tab') || true
[[ "$FL_CLOSE_ADDR" =~ ^t[0-9]+$ ]] || fail "M5: the new far tab's pane is not a core terminal: '$FL_CLOSE_PANE' '$FL_CLOSE_ADDR'"
FL_CLOSE_PID="$(fl_term_pid "$FL_CLOSE_ADDR")"
[[ -n "$FL_CLOSE_PID" ]] || fail "M5: the far terminal $FL_CLOSE_ADDR is not live"
fl_others() {
  fl_far_panes | awk -v pane="$FL_CLOSE_PANE" '$1 != pane' | while read -r pane _ address; do
    printf '%s %s %s\n' "$pane" "$address" "$(fl_term_pid "$address")"
  done
}
others_before="$(fl_others)"
[[ -n "$others_before" ]] || fail "M5: no other far pane to leave alone"
grep -q " t[0-9]* [0-9][0-9]*$" <<<"$others_before" || fail "M5: the other far panes have no live program: $others_before"
# A mark on the far pane's screen, so the view shows when it is that pane's tab.
python3 -I - "$FL_FAR_API" "$FL_CLOSE_PANE" "E3-CLOSE-$$" <<'PY' || fail "M5: could not type into the far pane"
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall((json.dumps({"id": "m5", "method": "pane.send_input", "params": {"pane_id": sys.argv[2], "text": "echo " + sys.argv[3], "keys": ["Enter"]}}) + "\n").encode())
assert "result" in json.loads(s.makefile("rb").readline())
PY
fl_view_has() { fl_tm capture-pane -p -t flview 2>/dev/null | grep -q "$1"; }
fl_tm send-keys -t flview C-b n
fl_wait "the view showing the new far tab's pane" 20 fl_view_has "E3-CLOSE-$$"
bridges="$(grep -c "args=--session $FL_SESSION remote-api-bridge --request" "$FL_FAR_LOG" || true)"
fl_tm send-keys -t flview C-b x
fl_close_gone() { ! fl_far_panes | awk '{print $1}' | grep -qxF "$FL_CLOSE_PANE"; }
fl_wait "the far pane $FL_CLOSE_PANE closed" 30 fl_close_gone
fl_program_ended() { [[ -z "$(fl_term_pid "$FL_CLOSE_ADDR")" ]] && ! ps -o pid= -p "$FL_CLOSE_PID" >/dev/null 2>&1; }
fl_wait "the far program $FL_CLOSE_PID ending" 15 fl_program_ended
[[ "$(grep -c "args=--session $FL_SESSION remote-api-bridge --request" "$FL_FAR_LOG")" -gt "$bridges" ]] \
  || fail "M5: the view's close did not go through the far bridge"
[[ "$(fl_others)" == "$others_before" ]] \
  || fail "M5: another far pane or program changed: before '$others_before', after '$(fl_others)'"
[[ "$(fl_cli_leases)" == 0 ]] || fail "M5: the close's lease outlived it: $(ls "$FL_DIR/leases")"
[[ "$(tail -n 1 "$FL_FAR_REQ_LOG" 2>/dev/null)" == pane.close_if_terminal ]] \
  || fail "#316 M1: the view's close reached the far herdr as '$(tail -n 1 "$FL_FAR_REQ_LOG" 2>/dev/null)', not pane.close_if_terminal"
echo "ok: E.3 M5 prefix x in the view closed far pane $FL_CLOSE_PANE ($FL_CLOSE_ADDR, pid $FL_CLOSE_PID) through the far bridge as pane.close_if_terminal; its program ended; the other far panes and programs are as they were; no lease left"

# --- #316 M1 live: a far pane whose program is respawned after the view's
# own check (its document and Herdr Core's record still showed the program
# the view attached to) and before the far herdr takes the request: the far
# herdr checks the program again in the same step as the close and refuses
# it (`terminal_mismatch`); the pane and its new program stay.
fl_far_api() {
  python3 -I - "$FL_FAR_API" "$1" "$2" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall((json.dumps({"id": "fl", "method": sys.argv[2], "params": json.loads(sys.argv[3])}) + "\n").encode())
print(json.dumps(json.loads(s.makefile("rb").readline())))
PY
}
fl_far_api tab.create '{"label":"rebind-316","focus":false}' >"$BASE/fl-rebind-tab.json" \
  || fail "#316: could not make the far tab"
fl_rebind_tab="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["tab"]["tab_id"])' "$BASE/fl-rebind-tab.json")" \
  || fail "#316: no tab in $(cat "$BASE/fl-rebind-tab.json")"
fl_rebind_pane() { fl_far_panes | awk -v tab="$fl_rebind_tab" '$2 == tab {print $1, $3}'; }
fl_rebind_ready() { [[ "$(fl_rebind_pane | awk '{print $2}')" =~ ^t[0-9]+$ ]]; }
fl_wait "the far rebind tab's core pane" 20 fl_rebind_ready
read -r FL_REBIND_PANE FL_REBIND_ADDR < <(fl_rebind_pane)
fl_far_api pane.send_input "{\"pane_id\":\"$FL_REBIND_PANE\",\"text\":\"echo REBIND-$$\",\"keys\":[\"Enter\"]}" >/dev/null \
  || fail "#316: could not type into the far pane"
for _ in 1 2 3 4 5 6 7 8; do
  fl_view_has "REBIND-$$" && break
  fl_tm send-keys -t flview C-b n
  sleep 1
done
fl_wait "the view showing the far rebind pane" 20 fl_view_has "REBIND-$$"
# The view must have attached it (a pane still attaching takes no action).
sleep 2
cat > "$FL_FAR_HOOK" <<HOOK
"$FL_PY" -I - "$FL_FAR_API" "$FL_REBIND_PANE" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall((json.dumps({"id": "hook", "method": "pane.respawn", "params": {"pane_id": sys.argv[2], "force": True}}) + "\\n").encode())
s.makefile("rb").readline()
PY
HOOK
replies="$(wc -l <"$FL_FAR_REPLY_LOG" 2>/dev/null | tr -d ' ' || echo 0)"
fl_tm send-keys -t flview C-b x
fl_hook_ran() { [[ ! -s "$FL_FAR_HOOK" ]]; }
fl_wait "the far wrapper's respawn just before the request" 30 fl_hook_ran
fl_refused() { tail -n +"$((replies + 1))" "$FL_FAR_REPLY_LOG" 2>/dev/null | grep -q '"code":"terminal_mismatch"'; }
fl_wait "the far herdr's terminal_mismatch" 30 fl_refused
[[ "$(tail -n 1 "$FL_FAR_REQ_LOG")" == pane.close_if_terminal ]] \
  || fail "#316: the refused close went as '$(tail -n 1 "$FL_FAR_REQ_LOG")'"
read -r still_pane still_addr < <(fl_rebind_pane) || true
[[ "$still_pane" == "$FL_REBIND_PANE" ]] || fail "#316: the far pane $FL_REBIND_PANE was closed though its program changed"
[[ "$still_addr" =~ ^t[0-9]+$ && "$still_addr" != "$FL_REBIND_ADDR" ]] \
  || fail "#316: the hook did not respawn the far pane ($FL_REBIND_ADDR, now '$still_addr')"
[[ -n "$(fl_term_pid "$still_addr")" ]] || fail "#316: the far pane's new program $still_addr is not live"
[[ "$(fl_cli_leases)" == 0 ]] || fail "#316: the refused close's lease outlived it: $(ls "$FL_DIR/leases")"
fl_tm kill-server >/dev/null 2>&1
fl_stop_view
echo "ok: #316 M1 a far pane respawned between the view's check and the request ($FL_REBIND_ADDR -> $still_addr) is refused by the far herdr (terminal_mismatch) and stays; no lease left"
# The far layout changed: the link's answer is read again, and checked
# against the far herdr's own, for the comparisons below.
fleet_cli --machine "$FT" pane list >"$BASE/fl-core.json" 2>"$BASE/fl-core.err" \
  || fail "--machine $FT pane list after the view's tab failed: $(cat "$BASE/fl-core.err")"
far_fl_herdr pane list >"$BASE/fl-local.json" 2>/dev/null || fail "the far herdr's own pane list failed"
same_json "$BASE/fl-core.json" "$BASE/fl-local.json" || fail "--machine $FT gave other JSON than the far herdr after the view's tab"

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
dormant_base="$(fl_log_count "REFUSE Dormant")"
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

fl_retries_backed_off "$dormant_base"
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
# The STARTs since the call began. Core's own retries keep coming, refused
# for no lease; one can land just before the call's dial, so the dial is the
# START that ran ssh (an EXEC with its pid), not the first one counted.
starts = [r for r in rows if r[2] == "START"][int(sys.argv[3]):]
execd = {r[1] for r in rows if r[2] == "EXEC"}
dialled = [r for r in starts if r[1] in execd]
assert len(dialled) == 1, (dial, dialled)
assert float(dial[0]) <= float(dialled[0][0]), (dial, dialled)
PY
[[ "$(fl_cli_leases)" == 0 ]] || fail "the --dial lease outlived the call"
# E.2b M7: the announcement at run time; M4: the dial used the grant up.
grep -q "herdr: --dial: opening ssh to $FT now" "$BASE/fl-dial.err" \
  || fail "--dial did not announce its dial on stderr: $(cat "$BASE/fl-dial.err")"
[[ "$(fl_log_count "GRANT used")" -gt 0 ]] || fail "core-link did not use the --dial grant: $(tail -n 8 "$FL_DIR/link.log")"
echo "ok: V32 --dial announced and wrote its lease before the one dial, which used it up, gave the far JSON, and removed the lease"

# --- 2.2: a link dropped during a --dial call is not redialled on its
# account: once connected the call's lease only keeps the link. A keep
# lease of the test's own holds through the window, so core's retries meet
# it, and only it.
drop_base="$(fl_log_count "REFUSE Dormant")"
fl_wait "$FT dormant before the drop" 20 fl_is unreachable
fl_retries_backed_off "$drop_base"
starts="$(fl_ssh_starts)"
settled="$(fl_log_count "SETTLED")"
fleet_cli_bg --machine "$FT" --dial pane wait-output "$FL_PANE" --match "never-printed-$$" --timeout 30000 \
  >"$BASE/fl-drop.out" 2>"$BASE/fl-drop.err"
drop_pid=$!
fl_wait "the --dial call's lease to keep the link only" 25 fl_lease_is_keep "$drop_pid"
fl_wait "$FT connected for the call" 10 fl_is connected
bridges="$(grep -c "args=--session $FL_SESSION remote-api-bridge" "$FL_FAR_LOG")"
fl_far_bridge_more() { [[ "$(grep -c "args=--session $FL_SESSION remote-api-bridge" "$FL_FAR_LOG")" -gt "$bridges" ]]; }
fl_wait "the call's request on the far side" 15 fl_far_bridge_more
# A link that settled (5 s up) cleared the retry budget, so the drop is a
# first failure that closes nothing: only the leases decide core's retry.
fl_wait "the call's link settled" 15 fl_more "SETTLED" "$settled"
python3 -I -c 'import os,sys,time
d = sys.argv[1]; tmp = os.path.join(d, ".cli-1.tmp")
open(tmp, "w").write(f"{time.time() + 60:.3f} 0 keep\n"); os.replace(tmp, os.path.join(d, "cli-1"))' "$FL_DIR/leases"
refusals="$(fl_log_count "REFUSE Dormant")"
ssh_pid="$(awk '/ SSH pid=.* machine=fleet-target /{for (i = 1; i <= NF; i++) if ($i ~ /^pid=/) p = substr($i, 5)} END {print p}' "$FL_SSH_LOG")"
[[ -n "$ssh_pid" ]] || fail "no ssh pid for the link"
kill -TERM "$ssh_pid"
code=0; wait "$drop_pid" || code=$?
[[ "$code" == 1 ]] || fail "the call over a dropped link ended $code, not 1: $(cat "$BASE/fl-drop.err")"
grep -q "it may have run there, and it is not sent again" "$BASE/fl-drop.err" \
  || fail "the dropped call does not say it may have run: $(cat "$BASE/fl-drop.err")"
[[ ! -s "$BASE/fl-drop.out" ]] || fail "the dropped call printed output"
fl_wait "core's retry refused as dormant" 40 fl_more "REFUSE Dormant" "$refusals"
sleep 3
[[ "$(fl_ssh_starts)" == $((starts + 1)) ]] || fail "the drop was redialled: $(( $(fl_ssh_starts) - starts )) ssh for one --dial"
rm -f "$FL_DIR/leases/cli-1"
echo "ok: 2.2 a link dropped during a --dial call: the call failed saying it may have run; core's retry met only keep leases and was refused; one ssh"

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
