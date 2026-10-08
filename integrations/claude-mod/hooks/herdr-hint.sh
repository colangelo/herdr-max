#!/bin/sh
# Part of the herdr-attention mod (fork issue 157).
# usage: herdr-hint.sh <socket> <pane_id> <question|permission|clear> <id> <seq> <ttl_ms>
# Sends one pane.report_hint request to herdr's socket. Exits 0 and does
# nothing without a socket, a pane or python3, so it is inert outside a herdr
# pane. Never prints on success.

[ -n "${1:-}" ] && [ -n "${2:-}" ] && [ -n "${3:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

python3 - "$@" <<'PY'
import json
import socket
import sys

socket_path, pane_id, kind, hint_id, seq, ttl_ms = (sys.argv[1:7] + [""] * 6)[:6]
params = {
    "pane_id": pane_id,
    "source": "herdr:claude-mod",
    "agent": "claude",
}
if seq.isdigit():
    params["seq"] = int(seq)
if kind == "clear":
    params["clear"] = True
else:
    params["kind"] = kind
    if hint_id:
        params["id"] = hint_id
    if ttl_ms.isdigit():
        params["ttl_ms"] = int(ttl_ms)
request = {"id": "herdr:claude-mod", "method": "pane.report_hint", "params": params}

client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.settimeout(1.0)
try:
    client.connect(socket_path)
    client.sendall((json.dumps(request) + "\n").encode())
    reply = client.recv(4096).decode(errors="replace")
except OSError as err:
    sys.stderr.write(f"herdr socket: {err}\n")
    raise SystemExit(1)
finally:
    client.close()

if '"error"' in reply:
    sys.stderr.write(reply[:200] + "\n")
    raise SystemExit(1)
PY
