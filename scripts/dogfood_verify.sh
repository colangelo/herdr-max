#!/usr/bin/env bash
# Fork: the check that makes a beta install "seamless dogfood" or says why not.
#
#   scripts/dogfood_verify.sh snapshot FILE    before the install: every pane and its agent session
#   scripts/dogfood_verify.sh verify FILE      after it: no pane or session lost, and every window
#                                              ac has open runs the server's build
#
# A live handoff moves only the server. The sidebar, input and remote keepalives are drawn and
# sent by the CLIENT, so a change there reaches ac only when each attached client runs the new
# build. On 2026-11-01 beta 180/181 were reported live while ac's main window still ran 174 and
# his --remote tab 179 (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/261).
# `verify` exits 0 only when nothing was lost AND every client matches; otherwise it names each
# window that lags and the exact command that brings it up to date.
#
# Exit: 0 seamless, 2 a pane or session was lost, 3 a client runs another build, 1 usage/error.
set -euo pipefail

bin="${HERDR_DOGFOOD_BIN:-herdr-beta}"

panes_json() {
    "$bin" pane list | jq -c '[.result.panes[] | {pane_id, session: (.agent_session.value // null)}] | sort_by(.pane_id)'
}

# The build a process runs, from the Cellar path of its mapped executable.
build_of() {
    local exe
    exe="$(lsof -p "$1" -a -d txt -Fn 2>/dev/null | sed -n 's/^n//p' | grep -m1 'herdr' || true)"
    if [[ "$exe" =~ /Cellar/[^/]+/([^/]+)/ ]]; then
        echo "${BASH_REMATCH[1]}"
    else
        echo "unknown(${exe:-no exe})"
    fi
}

# Attached TUI clients: a herdr-beta process on a terminal that is not the `--remote` launcher
# (its `client` child draws that tab) and not a one-shot CLI call.
clients() {
    ps -axo pid=,ppid=,tty=,command= | awk -v b="$bin" '
        $3 == "??" { next }
        {
            cmd = ""; for (i = 4; i <= NF; i++) cmd = cmd (i > 4 ? " " : "") $i
            n = split($4, path, "/"); exe = path[n]
            if (exe != b) next
            rest = substr(cmd, length($4) + 2)
            if (rest == "" || rest == "client" || rest ~ /^--session /) print $1, $2, rest
        }'
}

server_version() {
    "$bin" status server --json | jq -r '.result.version // .version // empty'
}

case "${1:-}" in
snapshot)
    [[ -n "${2:-}" ]] || { echo "usage: $0 snapshot FILE" >&2; exit 1; }
    panes_json > "$2"
    echo "dogfood: snapshot of $(jq length "$2") panes -> $2"
    ;;
verify)
    [[ -n "${2:-}" && -f "$2" ]] || { echo "usage: $0 verify SNAPSHOT_FILE" >&2; exit 1; }
    rc=0
    after="$(panes_json)"
    lost="$(jq -c --argjson after "$after" '[.[] | select(. as $p | $after | index($p) | not)]' "$2")"
    if [[ "$lost" != "[]" ]]; then
        echo "dogfood: LOST panes or sessions (pane_id, session): $lost"
        rc=2
    else
        echo "dogfood: all $(jq length "$2") panes and their sessions kept"
    fi

    server="$(server_version)"
    echo "dogfood: server runs $server"
    found=0
    while read -r pid ppid rest; do
        [[ -n "$pid" ]] || continue
        found=1
        build="$(build_of "$pid")"
        parent="$(ps -o command= -p "$ppid" 2>/dev/null || true)"
        if [[ "$rest" == "client" && "$parent" =~ --remote[=\ ]([^ ]+) ]]; then
            window="remote tab ${BASH_REMATCH[1]}"
            fix="detach it, then run: $bin --remote ${BASH_REMATCH[1]}"
        else
            window="main window${rest:+ ($rest)}"
            fix="detach it, then run: $bin${rest:+ $rest}"
        fi
        if [[ "$build" == "$server" ]]; then
            echo "dogfood: $window (pid $pid) runs $build: up to date"
        else
            echo "dogfood: $window (pid $pid) runs $build, NOT $server: $fix"
            [[ $rc -eq 0 ]] && rc=3
        fi
    done < <(clients)
    [[ $found -eq 1 ]] || echo "dogfood: no attached client found (nothing open on this Mac)"
    if [[ $rc -eq 0 ]]; then
        echo "dogfood: SEAMLESS"
    else
        echo "dogfood: NOT seamless (exit $rc): do not report this install as live for client-side changes"
    fi
    exit "$rc"
    ;;
*)
    echo "usage: $0 snapshot FILE | verify FILE" >&2
    exit 1
    ;;
esac
