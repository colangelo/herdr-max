"""Tests for integrations/claude-mod/hooks/herdr-hint.sh (fork issue 157): the
helper the Claude Code mod runs to send one pane.report_hint request to herdr's
socket."""

import json
import os
import socket
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "integrations/claude-mod/hooks/herdr-hint.sh"


class FakeHerdr:
    """A unix socket that answers one request and records it."""

    def __init__(self, reply: str = '{"id":"x","result":{"type":"ok"}}'):
        self.dir = tempfile.mkdtemp(prefix="herdr-hint-test-")
        self.path = os.path.join(self.dir, "s.sock")
        self.reply = reply
        self.request: dict = {}
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(self.path)
        self.server.listen(1)
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()

    def _serve(self):
        conn, _ = self.server.accept()
        with conn:
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(4096)
                if not chunk:
                    break
                data += chunk
            self.request = json.loads(data.decode())
            conn.sendall((self.reply + "\n").encode())

    def close(self):
        self.thread.join(timeout=3)
        self.server.close()
        try:
            os.unlink(self.path)
            os.rmdir(self.dir)
        except OSError:
            pass


def run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["sh", str(SCRIPT), *args], capture_output=True, text=True, timeout=10)


@unittest.skipUnless(os.name == "posix", "the claude-mod hint is a POSIX shell script on a unix socket")
class HerdrHintScript(unittest.TestCase):
    def test_a_hint_goes_to_the_socket_as_a_report_hint_request(self):
        herdr = FakeHerdr()
        try:
            done = run(herdr.path, "w1:p1", "question", "toolu_1", "1790000000000001", "15000")
        finally:
            herdr.close()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout, "")
        self.assertEqual(herdr.request["method"], "pane.report_hint")
        self.assertEqual(
            herdr.request["params"],
            {
                "pane_id": "w1:p1",
                "source": "herdr:claude-mod",
                "agent": "claude",
                "seq": 1790000000000001,
                "kind": "question",
                "id": "toolu_1",
                "ttl_ms": 15000,
            },
        )

    def test_a_clear_carries_no_kind_and_no_ttl(self):
        herdr = FakeHerdr()
        try:
            done = run(herdr.path, "w1:p1", "clear", "", "1790000000000002", "15000")
        finally:
            herdr.close()
        self.assertEqual(done.returncode, 0, done.stderr)
        params = herdr.request["params"]
        self.assertTrue(params["clear"])
        self.assertNotIn("kind", params)
        self.assertNotIn("ttl_ms", params)
        self.assertNotIn("id", params)

    def test_an_error_reply_fails_with_the_reply_on_stderr(self):
        herdr = FakeHerdr('{"id":"x","error":{"code":"pane_not_found","message":"no"}}')
        try:
            done = run(herdr.path, "w9:p9", "permission", "t", "1", "15000")
        finally:
            herdr.close()
        self.assertEqual(done.returncode, 1)
        self.assertIn("pane_not_found", done.stderr)

    def test_no_socket_listening_fails_quietly_on_stdout(self):
        done = run("/nonexistent/herdr.sock", "w1:p1", "question", "t", "1", "15000")
        self.assertEqual(done.returncode, 1)
        self.assertEqual(done.stdout, "")
        self.assertIn("herdr socket", done.stderr)

    def test_outside_a_herdr_pane_it_does_nothing(self):
        for args in [(), ("", ""), ("/tmp/s.sock",), ("/tmp/s.sock", "")]:
            done = run(*args)
            self.assertEqual(done.returncode, 0, args)
            self.assertEqual((done.stdout, done.stderr), ("", ""), args)


if __name__ == "__main__":
    unittest.main()
