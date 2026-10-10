from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "conventional_commits.py"


class ConventionalCommitsRangeTests(unittest.TestCase):
    """`--range` checks the commits a push brings in. Merge commits (2+ parents)
    are skipped; every commit they bring in is still checked (fork issue 210:
    https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/210)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        self.env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.invalid",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        }
        self.git("init", "-q", "-b", "master")
        self.commit("chore: base")
        self.base = self.rev("HEAD")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def git(self, *args: str) -> str:
        return subprocess.check_output(
            ["git", *args], cwd=self.repo, env=self.env, text=True
        ).strip()

    def rev(self, ref: str) -> str:
        return self.git("rev-parse", ref)

    def commit(self, subject: str) -> None:
        self.git("commit", "-q", "--allow-empty", "-m", subject)

    def merge(self, branch: str, subject: str) -> None:
        self.git("merge", "-q", "--no-ff", branch, "-m", subject)

    def check_range(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--range", f"{self.base}..HEAD"],
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
        )

    def test_a_merge_commit_subject_is_not_checked(self) -> None:
        self.git("switch", "-q", "-c", "topic")
        self.commit("fix: a real fix")
        self.git("switch", "-q", "master")
        self.merge("topic", "merge: topic brings in a real fix")
        result = self.check_range()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_bad_subject_inside_a_merge_still_fails(self) -> None:
        self.git("switch", "-q", "-c", "topic")
        self.commit("not conventional")
        self.git("switch", "-q", "master")
        self.merge("topic", "chore(train): merge topic")
        result = self.check_range()
        self.assertEqual(result.returncode, 1)
        self.assertIn("not conventional", result.stdout)
        self.assertNotIn("chore(train): merge topic", result.stdout)

    def test_a_bad_plain_commit_still_fails(self) -> None:
        self.commit("oops no type")
        result = self.check_range()
        self.assertEqual(result.returncode, 1)
        self.assertIn("oops no type", result.stdout)


if __name__ == "__main__":
    unittest.main()
