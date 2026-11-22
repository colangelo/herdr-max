from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "core_move_idle_panes.sh"

# A stand-in for the herdr CLI over a JSON state file ($FAKE_STATE): every
# call is logged; `pane respawn --if-idle` moves a pane onto the primary
# unless the state says it breaks, which leaves it where it was.
FAKE_HERDR = textwrap.dedent(
    r"""
    import json, os, sys
    path = os.environ["FAKE_STATE"]
    state = json.load(open(path))
    args = sys.argv[1:]
    state["calls"].append(args)
    def reply(result):
        json.dump(state, open(path, "w"))
        print(json.dumps({"id": "cli", "result": result}))
        raise SystemExit(0)
    panes = state["panes"]
    if args == ["pane", "list"]:
        reply({"panes": [dict(p["info"], pane_id=i) for i, p in panes.items()]})
    if args == ["agent", "list"]:
        reply({"agents": [{"pane_id": i, "name": p.get("name")} for i, p in panes.items()]})
    pane_id = args[3] if len(args) > 3 and args[2] == "--pane" else args[2]
    if pane_id not in panes:
        json.dump(state, open(path, "w"))
        print(json.dumps({"id": "cli", "error": {"code": "pane_not_found", "message": f"pane {pane_id} not found"}}))
        raise SystemExit(1)
    pane = panes[pane_id]
    if "replaced_by" in pane and args[:2] == ["pane", "get"]:
        pane["info"] = pane["replaced_by"]
    if args[:2] == ["pane", "get"]:
        reply({"pane": dict(pane["info"], pane_id=args[2])})
    if args[:2] == ["pane", "process-info"]:
        assert args[2] == "--pane", args  # herdr takes the pane as --pane <ID>
        reply({"process_info": {"pane_id": args[3], "foreground_processes": pane.get("processes", [])}})
    if args[:2] == ["pane", "respawn"] and "--check" in args:
        reply({"moved": False, "refusals": ["already_on_primary"] if pane.get("primary") else pane.get("refusals", [])})
    if args[:2] == ["pane", "respawn"] and "--if-idle" in args:
        if pane.get("refusals"):
            reply({"moved": False, "refusals": pane["refusals"]})
        if "move_answer" in pane:
            reply(pane["move_answer"])
        if pane.get("closes"):
            del panes[args[2]]
            reply({"moved": True, "refusals": []})
        pane["primary"] = not pane.get("breaks")
        reply({"moved": True, "refusals": []})
    raise SystemExit(f"unexpected call {args}")
    """
).lstrip()

# agent-bell's `inbox --peek --json`: the sessions in $FAKE_BELLS have one
# queued message each.
FAKE_BELL = textwrap.dedent(
    r"""
    import json, os, sys
    session = sys.argv[sys.argv.index("--session") + 1]
    queued = os.environ.get("FAKE_BELLS", "").split(",")
    print(json.dumps([{"msg_id": "m1"}] if session in queued else []))
    """
).lstrip()

PLAN = ["claude", "--resume", "sess-a", "--model", "opus", "--effort", "high",
        "--permission-mode", "auto", "--settings", "/s.json"]


def agent(name, session, **extra):
    plan = [word if word != "sess-a" else session for word in PLAN]
    return {
        "name": name,
        "info": {"agent": "claude", "agent_status": "idle", "cwd": "/w",
                 "agent_session": {"value": session, "restore_argv": plan}},
        "processes": [{"pid": 7, "name": "claude", "argv": plan}],
        **extra,
    }


@unittest.skipUnless(os.name == "posix", "the idle-move driver is a bash script")
class CoreMoveIdlePanesTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        for name, body in (("herdr", FAKE_HERDR), ("agent-bell", FAKE_BELL)):
            path = self.dir / name
            path.write_text(f"#!{sys.executable}\n{body}")
            path.chmod(0o755)
        self.state = self.dir / "state.json"

    def run_driver(self, panes, *args, bells=""):
        self.state.write_text(json.dumps({"panes": panes, "calls": []}))
        env = dict(os.environ, FAKE_STATE=str(self.state), FAKE_BELLS=bells)
        done = subprocess.run(
            ["bash", str(SCRIPT), "--herdr", str(self.dir / "herdr"),
             "--agent-bell", str(self.dir / "agent-bell"), "--settle", "0", *args],
            capture_output=True, text=True, env=env, timeout=60,
        )
        state = json.loads(self.state.read_text())
        return done, state

    def moves(self, state):
        return [call[2] for call in state["calls"]
                if call[:2] == ["pane", "respawn"] and "--check" not in call]

    def test_without_move_it_only_lists(self):
        done, state = self.run_driver({"w1-p1": agent("a", "sess-a")})
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("would move", done.stdout)
        self.assertEqual(self.moves(state), [])

    def test_the_canary_moves_first_and_the_first_failure_stops_the_run(self):
        panes = {
            "w1-p1": agent("canary", "sess-a"),
            "w1-p2": agent("breaks", "sess-b", breaks=True),
            "w1-p3": agent("later", "sess-c"),
        }
        done, state = self.run_driver(panes, "--move")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertEqual(self.moves(state), ["w1-p1", "w1-p2"])
        self.assertIn("moving w1-p1 (canary, canary)", done.stdout)
        self.assertIn("FAILED: not on the primary core yet", done.stdout)

    def test_a_changed_model_fails_the_check(self):
        moved = agent("a", "sess-a")
        moved["processes"][0]["argv"] = [w if w != "opus" else "sonnet" for w in PLAN]
        done, _ = self.run_driver({"w1-p1": moved}, "--move")
        self.assertEqual(done.returncode, 1, done.stdout)
        self.assertIn("--model 'sonnet', the plan says 'opus'", done.stdout)

    def test_a_helper_carrying_the_session_id_is_not_read_as_the_agent(self):
        # m4m, beta 209: the agent-bell listener runs first in the pane with
        # --session <id> and no --model; the check must read claude's argv.
        moved = agent("a", "sess-a")
        listener = {"pid": 5, "name": "python",
                    "argv": ["python", "agent-bell", "listen", "--session", "sess-a"]}
        moved["processes"].insert(0, listener)
        done, _ = self.run_driver({"w1-p1": moved}, "--move")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertNotIn("FAILED", done.stdout)

    def test_excluded_panes_are_never_touched(self):
        panes = {"w1-p1": agent("manager-lab-hlp", "sess-a"), "w1-p2": agent("b", "sess-b")}
        done, state = self.run_driver(panes, "--move", "--exclude", "*-hlp")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertFalse(any("w1-p1" in call for call in state["calls"]))
        self.assertIn("skipped: --exclude *-hlp", done.stdout)
        self.assertEqual(self.moves(state), ["w1-p2"])

    def test_server_refusals_and_queued_bells_are_reported_not_moved(self):
        panes = {
            "w1-p1": agent("x", "sess-a", refusals=["excluded"]),
            "w1-p2": agent("y", "sess-b"),
            "w1-p3": agent("z", "sess-c", primary=True),
        }
        done, state = self.run_driver(panes, "--move", bells="sess-b")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(self.moves(state), [])
        self.assertIn("refused: excluded", done.stdout)
        self.assertIn("skipped: 1 bell(s) queued", done.stdout)
        self.assertNotIn("w1-p3", done.stdout)

    def test_allow_queued_moves_a_pane_whose_only_wait_is_a_queued_bell(self):
        # The bell waits in agent-bell's durable inbox, keyed by session id,
        # and the moved agent resumes the same session.
        done, state = self.run_driver({"w1-p1": agent("y", "sess-b")},
                                      "--move", "--allow-queued", bells="sess-b")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(self.moves(state), ["w1-p1"])

    # Fork issue 301: a move that answers anything but moved stops the run,
    # visibly, before the next pane, refusals or not.
    def test_a_move_not_moved_stops_the_run_before_the_next_pane(self):
        for answer in ({"moved": False, "refusals": []},
                       {"moved": False, "refusals": ["core_unreachable"]}):
            panes = {
                "w1-p1": agent("canary", "sess-a", move_answer=answer),
                "w1-p2": agent("later", "sess-b"),
            }
            done, state = self.run_driver(panes, "--move")
            self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
            self.assertEqual(self.moves(state), ["w1-p1"])
            self.assertIn("FAILED: w1-p1 was not moved", done.stdout)
            expected = ", ".join(answer["refusals"]) or "no refusal given"
            self.assertIn(f"({expected})", done.stdout)
            self.assertIn("stopped at the first failure", done.stdout)

    def test_a_pane_gone_after_its_move_stops_the_run(self):
        panes = {
            "w1-p1": agent("canary", "sess-a", closes=True),
            "w1-p2": agent("later", "sess-b"),
        }
        done, state = self.run_driver(panes, "--move")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertEqual(self.moves(state), ["w1-p1"])
        self.assertIn("FAILED: w1-p1: the pane cannot be read, so it may be gone", done.stdout)

    def test_a_pane_holding_another_agent_than_listed_is_not_moved(self):
        other = agent("someone-else", "sess-z")["info"]
        panes = {
            "w1-p1": agent("canary", "sess-a", replaced_by=other),
            "w1-p2": agent("later", "sess-b"),
        }
        done, state = self.run_driver(panes, "--move")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        self.assertEqual(self.moves(state), [])
        self.assertIn("before its move, the pane now holds session 'sess-z'", done.stdout)

    def test_arguments(self):
        for args in (["--exclude"], ["--bogus"], ["--settle", "soon"]):
            done = subprocess.run(["bash", str(SCRIPT), *args], capture_output=True, text=True)
            self.assertEqual(done.returncode, 2, args)
            self.assertIn("usage:", done.stderr, args)
        done = subprocess.run(["bash", str(SCRIPT), "--help"], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0)


if __name__ == "__main__":
    unittest.main()
