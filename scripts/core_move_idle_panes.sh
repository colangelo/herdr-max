#!/usr/bin/env bash
# Moves idle panes onto the primary herdr-core session, one at a time
# (openspec/changes/core-phase3 D9, V25). The server decides each move
# (`herdr pane respawn --if-idle`); this script picks the order and checks
# each moved pane.
#
#   core_move_idle_panes.sh [--move] [--exclude GLOB]... [--herdr BIN]
#                           [--agent-bell CMD] [--settle SECONDS]
#
# Without --move it only lists each pane off the primary core and why the
# server would refuse it. With --move it moves one canary first, then the
# rest, and stops at the first pane that fails its check. A moved agent must
# keep its conversation (session id), its name, its model, effort, permission
# mode and settings; a moved shell its directory. Before an agent moves,
# agent-bell must report no queued message for its session. --exclude adds
# names (glob, matched against the agent name) for this run only; the server
# refuses panes matched by `[experimental] core_pty_panes` itself.
#
# Exit: 0 when every move checked out (or nothing moved), 1 at the first
# failed check, 2 for bad arguments or a server that cannot be read.
exec python3 -I - "$@" <<'PY'
import fnmatch
import json
import subprocess
import sys
import time

USAGE = ("usage: core_move_idle_panes.sh [--move] [--exclude GLOB]... [--herdr BIN] "
         "[--agent-bell CMD] [--settle SECONDS]")
PLAN_FLAGS = ("--model", "--effort", "--permission-mode", "--settings")


def parse(argv):
    options = {"move": False, "exclude": [], "herdr": "herdr", "bell": "agent-bell", "settle": 120.0}
    args = list(argv)
    while args:
        arg = args.pop(0)
        if arg == "--move":
            options["move"] = True
        elif arg in ("--exclude", "--herdr", "--agent-bell", "--settle"):
            if not args or args[0].startswith("--"):
                raise SystemExit(f"{arg} needs a value\n{USAGE}")
            value = args.pop(0)
            if arg == "--exclude":
                options["exclude"].append(value)
            elif arg == "--herdr":
                options["herdr"] = value
            elif arg == "--agent-bell":
                options["bell"] = value
            else:
                try:
                    options["settle"] = float(value)
                except ValueError:
                    raise SystemExit(f"--settle needs seconds, not {value!r}\n{USAGE}")
        elif arg in ("-h", "--help"):
            print(USAGE)
            raise SystemExit(0)
        else:
            raise SystemExit(f"unknown argument {arg!r}\n{USAGE}")
    return options


def fail_usage(message):
    print(message, file=sys.stderr)
    raise SystemExit(2)


class Herdr:
    def __init__(self, binary):
        self.binary = binary

    def call(self, *args):
        done = subprocess.run([self.binary, *args], capture_output=True, text=True, timeout=60)
        try:
            reply = json.loads(done.stdout)
        except ValueError:
            raise RuntimeError(f"{self.binary} {' '.join(args)}: {done.stderr.strip() or done.stdout.strip()}")
        if "error" in reply:
            raise RuntimeError(f"{self.binary} {' '.join(args)}: {reply['error']}")
        return reply["result"]

    def panes(self):
        return self.call("pane", "list")["panes"]

    def names(self):
        return {agent["pane_id"]: agent.get("name") for agent in self.call("agent", "list")["agents"]}

    def pane(self, pane_id):
        return self.call("pane", "get", pane_id)["pane"]

    def refusals(self, pane_id):
        return self.call("pane", "respawn", pane_id, "--if-idle", "--check")["refusals"]

    def move(self, pane_id):
        return self.call("pane", "respawn", pane_id, "--if-idle")

    def processes(self, pane_id):
        info = self.call("pane", "process-info", pane_id)["process_info"]
        return info.get("nested_foreground_processes") or info.get("foreground_processes") or []


def flag_value(argv, flag):
    for index, word in enumerate(argv or []):
        if word == flag and index + 1 < len(argv):
            return argv[index + 1]
        if word.startswith(flag + "="):
            return word.split("=", 1)[1]
    return None


def session_of(pane):
    return (pane.get("agent_session") or {}).get("value")


def restore_of(pane):
    return (pane.get("agent_session") or {}).get("restore_argv") or []


def bells_queued(bell, session):
    """None when agent-bell reports none for the session, else why the pane waits."""
    try:
        done = subprocess.run([bell, "inbox", "--session", session, "--peek", "--json"],
                              capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as err:
        return f"agent-bell could not be asked ({err})"
    if done.returncode != 0:
        return f"agent-bell could not tell ({done.stderr.strip() or done.returncode})"
    try:
        waiting = json.loads(done.stdout)
    except ValueError:
        return "agent-bell answered something else than a list"
    if not isinstance(waiting, list):
        return "agent-bell answered something else than a list"
    return f"{len(waiting)} bell(s) queued" if waiting else None


def check_agent(herdr, pane_id, before, name):
    """None once the moved agent kept everything, else what differs."""
    pane = herdr.pane(pane_id)
    if herdr.refusals(pane_id) != ["already_on_primary"]:
        return "not on the primary core yet"
    if pane.get("agent_status") in (None, "unknown"):
        return "its agent has not reported yet"
    if session_of(pane) != session_of(before):
        return f"session {session_of(pane)!r}, was {session_of(before)!r}"
    if herdr.names().get(pane_id) != name:
        return f"name {herdr.names().get(pane_id)!r}, was {name!r}"
    plan = restore_of(before)
    session = session_of(before)
    argv = next((process.get("argv") for process in herdr.processes(pane_id)
                 if session and session in (process.get("argv") or [])), None)
    if argv is None:
        return "no process carries its session id"
    for flag in PLAN_FLAGS:
        wanted = flag_value(plan, flag)
        if wanted is not None and flag_value(argv, flag) != wanted:
            return f"{flag} {flag_value(argv, flag)!r}, the plan says {wanted!r}"
    model_before = flag_value(plan, "--model")
    model_after = flag_value(restore_of(pane), "--model")
    if model_before is not None and model_after != model_before:
        return f"the status line shows model {model_after!r}, was {model_before!r}"
    return None


def check_shell(herdr, pane_id, before):
    if herdr.refusals(pane_id) != ["already_on_primary"]:
        return "not on the primary core yet"
    cwd = herdr.pane(pane_id).get("cwd")
    if cwd != before.get("cwd"):
        return f"directory {cwd!r}, was {before.get('cwd')!r}"
    return None


def settle(check, seconds):
    deadline = time.monotonic() + seconds
    while True:
        problem = check()
        if problem is None or time.monotonic() >= deadline:
            return problem
        time.sleep(1)


def main():
    try:
        options = parse(sys.argv[1:])
    except SystemExit as stop:
        if stop.code in (0, None):
            raise
        fail_usage(stop.code)
    herdr = Herdr(options["herdr"])
    try:
        panes = herdr.panes()
        names = herdr.names()
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as err:
        fail_usage(f"cannot read the herdr server: {err}")

    summary = []
    candidates = []
    for pane in panes:
        pane_id = pane["pane_id"]
        name = names.get(pane_id)
        glob = next((pattern for pattern in options["exclude"]
                     if name and fnmatch.fnmatchcase(name, pattern)), None)
        if glob:
            summary.append((pane_id, name, f"skipped: --exclude {glob}"))
            continue
        refusals = herdr.refusals(pane_id)
        if refusals == ["already_on_primary"]:
            continue
        if refusals:
            summary.append((pane_id, name, "refused: " + ", ".join(refusals)))
            continue
        candidates.append(pane)

    failed = False
    moved = 0
    for pane in candidates:
        pane_id = pane["pane_id"]
        name = names.get(pane_id)
        if not options["move"]:
            summary.append((pane_id, name, "would move"))
            continue
        before = herdr.pane(pane_id)
        agent = bool(before.get("agent"))
        if agent:
            session = session_of(before)
            waiting = bells_queued(options["bell"], session) if session else "no session id"
            if waiting:
                summary.append((pane_id, name, f"skipped: {waiting}"))
                continue
        role = "canary" if moved == 0 else "next"
        print(f"moving {pane_id} ({name or 'unnamed'}, {role})", flush=True)
        answer = herdr.move(pane_id)
        if not answer.get("moved"):
            summary.append((pane_id, name, "refused: " + ", ".join(answer.get("refusals") or [])))
            continue
        moved += 1
        if agent:
            problem = settle(lambda: check_agent(herdr, pane_id, before, name), options["settle"])
        else:
            problem = settle(lambda: check_shell(herdr, pane_id, before), options["settle"])
        if problem:
            summary.append((pane_id, name, f"FAILED: {problem}"))
            failed = True
            break
        summary.append((pane_id, name, "moved"))

    print("summary:")
    for pane_id, name, outcome in summary:
        print(f"  {pane_id}\t{name or '-'}\t{outcome}")
    if failed:
        print("stopped at the first failed check; the panes after it were not moved")
    raise SystemExit(1 if failed else 0)


main()
PY
