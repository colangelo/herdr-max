from __future__ import annotations

import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "beta_publish.sh"
BETA_WORKFLOW = ROOT / ".github" / "workflows" / "beta.yml"

# A stand-in for `gh`. It appends each invocation to $GH_LOG and answers from
# a tiny model of the repository kept in $GH_STATE: which tags and releases
# exist, and which command should fail (and how).
FAKE_GH = textwrap.dedent(
    r"""
    #!/usr/bin/env python3
    import json, os, shutil, sys
    args = sys.argv[1:]
    state_path = os.environ["GH_STATE"]
    state = json.load(open(state_path))
    with open(os.environ["GH_LOG"], "a") as log:
        log.write(" ".join(args) + "\n")
    line = " ".join(args)
    for pattern, message in state.get("fail", {}).items():
        if pattern in line:
            print(message, file=sys.stderr)
            sys.exit(1)

    def save():
        json.dump(state, open(state_path, "w"))

    def opt(name):
        return args[args.index(name) + 1]

    if args[0] == "api":
        path = next(a for a in args[1:] if a.startswith("repos/"))
        method = opt("--method") if "--method" in args else "GET"
        if "/contents/.github/workflows" in path:
            ref = path.split("ref=")[1]
            files = state["workflows"].get(ref, state["workflows"]["master"])
            print("\n".join(f"{n} {s}" for n, s in files))
        elif "/git/ref/tags/" in path:
            if path.rsplit("/", 1)[1] not in state["tags"]:
                print("gh: Not Found (HTTP 404)", file=sys.stderr)
                sys.exit(1)
        elif "/releases/tags/" in path:
            if path.rsplit("/", 1)[1] not in state["releases"]:
                print("gh: Not Found (HTTP 404)", file=sys.stderr)
                sys.exit(1)
        elif method == "DELETE" and "/git/refs/tags/" in path:
            state["tags"].pop(path.rsplit("/", 1)[1])
        elif method in ("PATCH", "POST"):
            sha = next(a for a in args if a.startswith("sha="))[4:]
            state["tags"]["beta"] = sha
        save()
    elif args[:2] == ["release", "list"]:
        print("\n".join(t for t in state["releases"] if t.startswith("beta-staging-")))
    elif args[:2] == ["release", "create"]:
        tag = args[2]
        state["tags"][tag] = opt("--target")
        assets = [a for a in args[3:] if os.path.isfile(a)]
        state["releases"][tag] = assets
        save()
    elif args[:2] == ["release", "download"]:
        tag, out = args[2], opt("--dir")
        for asset in state["releases"][tag]:
            shutil.copy(asset, out)
        corrupt = state.get("corrupt_download")
        if corrupt == tag:
            for name in os.listdir(out):
                open(os.path.join(out, name), "a").write("x")
    elif args[:2] == ["release", "delete"]:
        tag = args[2]
        state["releases"].pop(tag)
        if "--cleanup-tag" in args:
            state["tags"].pop(tag, None)
        save()
    elif args[:2] == ["release", "edit"]:
        tag, new = args[2], opt("--tag")
        state["releases"][new] = state["releases"].pop(tag)
        save()
    """
).lstrip()


@unittest.skipUnless(os.name == "posix", "beta publish runs a bash script")
class BetaPublishTests(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.log = self.dir / "gh.log"
        self.state_path = self.dir / "state.json"
        self.old_asset = self.dir / "old-herdr"
        self.old_asset.write_text("old build")
        self.assets = []
        for name in ("herdr-macos-aarch64", "herdr-macos-x86_64"):
            path = self.dir / name
            path.write_text(f"new {name}")
            self.assets.append(str(path))
        self.state = {
            "tags": {"beta": "oldsha"},
            "releases": {"beta": [str(self.old_asset)]},
            "workflows": {"master": [["beta.yml", "aaa"], ["ci.yml", "bbb"]]},
            "fail": {},
        }
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "GH_LOG": str(self.log),
            "GH_STATE": str(self.state_path),
            "GITHUB_REPOSITORY": "colangelo/herdr-max",
            "GITHUB_RUN_ID": "42",
            "BETA_VERSION": "0.8.2-ac-beta.93-pirlo",
            "TARGET_SHA": "newsha",
            "SOURCE_REF": "master",
            "BETA_NOTES": "notes",
        }

    def run_publish(self) -> subprocess.CompletedProcess[str]:
        import json

        self.state_path.write_text(json.dumps(self.state))
        result = subprocess.run(
            [str(SCRIPT), *self.assets],
            env=self.env,
            capture_output=True,
            text=True,
        )
        self.state = json.loads(self.state_path.read_text())
        return result

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def assert_old_beta_intact(self) -> None:
        self.assertEqual(self.state["releases"].get("beta"), [str(self.old_asset)])
        self.assertEqual(self.state["tags"].get("beta"), "oldsha")

    def test_success_moves_beta_to_the_new_assets_and_drops_staging(self) -> None:
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.state["releases"], {"beta": self.assets})
        self.assertEqual(self.state["tags"], {"beta": "newsha"})
        calls = self.calls()
        create = next(i for i, c in enumerate(calls) if c.startswith("release create"))
        delete = next(i for i, c in enumerate(calls) if c.startswith("release delete beta"))
        self.assertLess(create, delete, "the new build must exist before beta is deleted")
        self.assertIn("release create beta-staging-42", calls[create])
        self.assertNotIn("--cleanup-tag", calls[delete])

    def test_refused_create_leaves_the_old_beta_untouched(self) -> None:
        self.state["fail"] = {
            "release create": "HTTP 403: Resource not accessible by integration"
        }
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("HTTP 403", result.stderr)
        self.assert_old_beta_intact()
        self.assertFalse(any(c.startswith("release delete beta") for c in self.calls()))

    def test_workflow_drift_is_refused_before_anything_is_touched(self) -> None:
        self.state["workflows"]["newsha"] = [["beta.yml", "zzz"], ["ci.yml", "bbb"]]
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from master", result.stderr)
        self.assert_old_beta_intact()
        self.assertFalse(any(c.startswith("release") for c in self.calls()))

    def test_corrupt_staging_download_stops_before_beta_moves(self) -> None:
        self.state["corrupt_download"] = "beta-staging-42"
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("byte-for-byte", result.stderr)
        self.assert_old_beta_intact()

    def test_refused_tag_move_leaves_the_old_beta_release_serving(self) -> None:
        self.state["fail"] = {"git/refs/tags/beta": "HTTP 403: Resource not accessible"}
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assert_old_beta_intact()

    def test_failed_swap_names_the_staging_release_and_the_fix(self) -> None:
        self.state["fail"] = {"release edit": "HTTP 422"}
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("still published as beta-staging-42", result.stderr)
        self.assertIn("gh release edit beta-staging-42", result.stderr)
        self.assertEqual(self.state["releases"].get("beta-staging-42"), self.assets)

    def test_lookup_errors_are_not_mistaken_for_missing(self) -> None:
        self.state["fail"] = {"releases/tags/beta": "HTTP 500: server error"}
        result = self.run_publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not look up release beta", result.stderr)
        self.assertEqual(self.state["releases"].get("beta"), [str(self.old_asset)])

    def test_first_publish_creates_the_beta_tag(self) -> None:
        self.state["tags"] = {}
        self.state["releases"] = {}
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.state["tags"], {"beta": "newsha"})
        self.assertTrue(any("--method POST" in c for c in self.calls()))

    def test_leftover_staging_releases_are_cleared(self) -> None:
        self.state["releases"]["beta-staging-7"] = [str(self.old_asset)]
        self.state["tags"]["beta-staging-7"] = "stale"
        result = self.run_publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("beta-staging-7", self.state["releases"])
        self.assertNotIn("beta-staging-7", self.state["tags"])


class BetaWorkflowPublishTests(unittest.TestCase):
    def setUp(self) -> None:
        self.text = BETA_WORKFLOW.read_text()

    def test_publish_goes_through_the_safe_script(self) -> None:
        self.assertIn("scripts/beta_publish.sh", self.text)

    def test_the_old_silenced_delete_is_gone(self) -> None:
        self.assertNotIn("2>/dev/null || true", self.text)
        self.assertNotIn("gh release delete beta", self.text)

    def test_homebrew_waits_for_the_publish(self) -> None:
        homebrew = self.text[self.text.index("  update-homebrew:") :]
        self.assertIn("needs: [prep, publish]", homebrew)


if __name__ == "__main__":
    unittest.main()
