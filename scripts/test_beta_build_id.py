from __future__ import annotations

import os
import re
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "beta_version.sh"
FORMULA = ROOT / "scripts" / "beta_formula.sh"
LOCAL = ROOT / "scripts" / "beta_local.sh"
BETA_WORKFLOW = ROOT / ".github" / "workflows" / "beta.yml"
BETA_LINUX_WORKFLOW = ROOT / ".github" / "workflows" / "beta-linux.yml"

# A stand-in for `gh release view beta`: prints $FAKE_BETA_TITLE, or fails
# with $FAKE_GH_ERROR.
FAKE_GH = textwrap.dedent(
    r"""
    #!/usr/bin/env bash
    if [ -n "${FAKE_GH_ERROR:-}" ]; then
      echo "$FAKE_GH_ERROR" >&2
      exit 1
    fi
    echo "$FAKE_BETA_TITLE"
    """
).lstrip()

ARM = "a" * 64
X86 = "b" * 64


@unittest.skipUnless(os.name == "posix", "the beta scripts are bash")
class BetaBuildIdTests(unittest.TestCase):
    """Guard the beta build-id scheme: a monotonic number plus a Juventus
    surname codename (e.g. ``0.7.5-ac-beta.45-zakaria``).

    The number leads the id so ``brew upgrade herdr-beta`` stays monotonic. It
    is the larger of the GitHub run number and the published beta's number plus
    one, so a beta built on our own Mac and a later GitHub run never share a
    number (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/235). A
    retired scheme stamped a raw UTC timestamp; this suite fails if that comes
    back. Rationale + the "do not revert" note live in
    ``.claude/skills/herdr-release/SKILL.md``.
    """

    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        gh = self.dir / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.dir}:{os.environ['PATH']}",
            "FAKE_BETA_TITLE": "Beta 0.9.5-ac-beta.166-kulusevski",
        }
        self.env.pop("CODENAME", None)

    def ids(self, *args: str, **env: str) -> dict[str, str]:
        result = subprocess.run(
            [str(SCRIPT), *args],
            cwd=ROOT,
            env={**self.env, **env},
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return dict(line.split("=", 1) for line in result.stdout.splitlines())

    def fails(self, *args: str, **env: str) -> str:
        result = subprocess.run(
            [str(SCRIPT), *args],
            cwd=ROOT,
            env={**self.env, **env},
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(result.stdout, "")
        return result.stderr

    def test_a_local_build_takes_the_number_after_the_published_beta(self) -> None:
        self.assertTrue(self.ids()["build_id"].startswith("167-"))

    def test_a_github_run_ahead_of_the_published_beta_keeps_its_run_number(self) -> None:
        self.assertTrue(self.ids("300")["build_id"].startswith("300-"))

    def test_a_github_run_behind_a_local_beta_skips_past_it(self) -> None:
        self.assertTrue(self.ids("120")["build_id"].startswith("167-"))

    def test_no_published_beta_starts_from_the_run_number(self) -> None:
        missing = {"FAKE_GH_ERROR": "release not found"}
        self.assertTrue(self.ids("5", **missing)["build_id"].startswith("5-"))
        self.assertTrue(self.ids(**missing)["build_id"].startswith("1-"))

    def test_any_other_gh_failure_stops_instead_of_reusing_a_number(self) -> None:
        err = self.fails(FAKE_GH_ERROR="HTTP 502: Bad Gateway")
        self.assertIn("could not read the published beta", err)

    def test_a_title_without_a_number_stops(self) -> None:
        self.assertIn("no -ac-beta.N", self.fails(FAKE_BETA_TITLE="Beta"))

    def test_version_string_combines_base_channel_and_build_id(self) -> None:
        base = re.search(r'^version = "(.*)"', (ROOT / "Cargo.toml").read_text(), re.M)
        assert base is not None
        ids = self.ids()
        self.assertEqual(ids["version"], f"{base.group(1)}-ac-beta.{ids['build_id']}")
        self.assertRegex(ids["build_id"], r"^\d+-[a-z]+$")

    def test_build_id_is_not_derived_from_a_timestamp(self) -> None:
        # The retired `date -u +%Y%m%d%H%M%S` form orders only by wall clock
        # and reads as an opaque blob. Fail loudly if it returns.
        for path in (SCRIPT, BETA_WORKFLOW):
            text = path.read_text()
            self.assertIsNone(re.search(r"BUILD_ID=\$\(\s*date\b", text), path)
            self.assertNotIn("%Y%m%d", text)

    def test_codename_pool_is_present_and_valid(self) -> None:
        match = re.search(r"NAMES=\(([^)]*)\)", SCRIPT.read_text())
        assert match is not None, "beta_version.sh must define a NAMES=(...) pool"
        names = match.group(1).split()
        self.assertGreaterEqual(len(names), 20)
        self.assertEqual([n for n in names if not re.fullmatch(r"[a-z]+", n)], [])

    def test_codename_can_be_pinned_and_is_validated(self) -> None:
        self.assertEqual(self.ids(CODENAME="pirlo")["build_id"], "167-pirlo")
        self.assertIn("is not in the pool", self.fails(CODENAME="notaplayer"))

    def test_the_workflow_and_the_local_build_share_the_script(self) -> None:
        workflow = BETA_WORKFLOW.read_text()
        self.assertIn('scripts/beta_version.sh "${RUN_NUMBER}"', workflow)
        self.assertIn("RUN_NUMBER: ${{ github.run_number }}", workflow)
        self.assertIn("CODENAME: ${{ inputs.codename }}", workflow)
        self.assertNotIn("NAMES=(", workflow)
        self.assertIn("beta_version.sh", LOCAL.read_text())


@unittest.skipUnless(os.name == "posix", "the beta scripts are bash")
class BetaFormulaTests(unittest.TestCase):
    def formula(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(FORMULA), *args], capture_output=True, text=True
        )

    def test_a_github_beta_serves_both_macs(self) -> None:
        out = self.formula("0.9.5-ac-beta.170-pirlo", ARM, X86)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn('version "0.9.5-ac-beta.170-pirlo"', out.stdout)
        self.assertIn("if Hardware::CPU.arm?", out.stdout)
        self.assertIn(f'sha256 "{ARM}"', out.stdout)
        self.assertIn(f'sha256 "{X86}"', out.stdout)
        self.assertIn("releases/download/beta/herdr-macos-x86_64", out.stdout)
        self.assertNotIn("depends_on arch", out.stdout)

    def test_a_local_beta_is_apple_silicon_only(self) -> None:
        out = self.formula("0.9.5-ac-beta.171-vidal", ARM)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("depends_on arch: :arm64", out.stdout)
        self.assertIn("releases/download/beta/herdr-macos-aarch64", out.stdout)
        self.assertNotIn("x86_64", out.stdout)
        self.assertNotIn("Hardware::CPU", out.stdout)

    def test_both_shapes_keep_the_install_and_the_test(self) -> None:
        for args in ((ARM,), (ARM, X86)):
            out = self.formula("1.0.0-ac-beta.1-pirlo", *args).stdout
            self.assertTrue(out.startswith("class HerdrBeta < Formula\n"))
            self.assertIn('bin.install binary => "herdr-beta"', out)
            self.assertIn('assert_match "herdr #{version}"', out)
            self.assertTrue(out.endswith("end\n"))

    def test_a_bad_checksum_or_arity_is_refused(self) -> None:
        self.assertEqual(self.formula("v", "nothex").returncode, 2)
        self.assertEqual(self.formula("v", ARM, "short").returncode, 2)
        self.assertEqual(self.formula("v").returncode, 2)

    def test_the_workflow_writes_the_formula_through_the_script(self) -> None:
        workflow = BETA_WORKFLOW.read_text()
        self.assertIn(
            '../scripts/beta_formula.sh "${BETA_VERSION}" "${DARWIN_ARM64}" "${DARWIN_AMD64}"',
            workflow,
        )
        self.assertNotIn("class HerdrBeta", workflow)


class BetaLinuxWorkflowTests(unittest.TestCase):
    """Linux binaries of the published beta (fork issue 239): same commit and
    version string, both static musl targets, uploaded only onto that beta."""

    def setUp(self) -> None:
        self.text = BETA_LINUX_WORKFLOW.read_text()

    def test_builds_both_static_linux_targets_on_the_beta_channel(self) -> None:
        self.assertIn("target: x86_64-unknown-linux-musl", self.text)
        self.assertIn("target: aarch64-unknown-linux-musl", self.text)
        self.assertIn("HERDR_BUILD_CHANNEL: ac-beta", self.text)
        self.assertIn("HERDR_BUILD_ID: ${{ inputs.build_id }}", self.text)
        self.assertIn("ref: ${{ inputs.ref }}", self.text)

    def test_checks_the_version_string(self) -> None:
        self.assertIn('expected \'herdr ${BASE}-ac-beta.${HERDR_BUILD_ID}\'', self.text)

    def test_uploads_only_onto_its_own_beta_with_checksums(self) -> None:
        self.assertIn('WANT="Beta ${BASE}-ac-beta.${BUILD_ID}"', self.text)
        self.assertIn("nothing uploaded", self.text)
        self.assertIn("gh release upload beta", self.text)
        self.assertIn("sha256sum", self.text)
        self.assertNotIn("gh release delete", self.text)

    def test_inputs_reach_the_shell_through_env_only(self) -> None:
        for line in self.text.splitlines():
            if "${{ inputs." in line:
                self.assertNotIn("run:", line)


if __name__ == "__main__":
    unittest.main()
