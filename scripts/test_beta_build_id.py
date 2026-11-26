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
    """Guard the herdr-max version scheme (fork issue 330): the version is
    Cargo.toml's plain semver, with no ``-beta.N`` and no codename, and the build
    id is the commit date plus short sha (``20261126-ab12cd3``).

    A published version is never built again: ``brew upgrade`` moves on a higher
    version only, so the script refuses one that is already the published one
    (or lower). The last herdr-beta builds were ``0.9.5-ac-beta.N-<codename>``
    (N up to 222); 0.10.0 sorts above all of them. Rationale lives in
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
            "FAKE_BETA_TITLE": "Beta 0.9.5-ac-beta.222-bonucci",
        }
        self.env.pop("BETA_ALLOW_SAME_VERSION", None)

    def run_script(self, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(SCRIPT)],
            cwd=ROOT,
            env={**self.env, **env},
            capture_output=True,
            text=True,
        )

    def ids(self, **env: str) -> dict[str, str]:
        result = self.run_script(**env)
        self.assertEqual(result.returncode, 0, result.stderr)
        return dict(line.split("=", 1) for line in result.stdout.splitlines())

    def fails(self, **env: str) -> str:
        result = self.run_script(**env)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(result.stdout, "")
        return result.stderr

    def cargo_version(self) -> str:
        base = re.search(r'^version = "(.*)"', (ROOT / "Cargo.toml").read_text(), re.M)
        assert base is not None
        return base.group(1)

    def test_the_version_is_cargos_plain_semver(self) -> None:
        ids = self.ids()
        self.assertEqual(ids["version"], self.cargo_version())
        self.assertRegex(ids["version"], r"^\d+\.\d+\.\d+$")
        self.assertNotIn("beta", ids["version"])

    def test_the_build_id_is_the_commit_date_and_short_sha(self) -> None:
        build_id = self.ids()["build_id"]
        self.assertRegex(build_id, r"^\d{8}-[0-9a-f]{7}$")
        head = subprocess.run(
            ["git", "rev-parse", "--short=7", "HEAD"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout.strip()
        self.assertTrue(build_id.endswith(head))

    def test_the_first_herdr_max_passes_over_the_last_herdr_beta(self) -> None:
        # 0.10.0 against 0.9.5-ac-beta.222: a version sort, not a text sort.
        self.assertEqual(self.run_script().returncode, 0)

    def test_a_published_version_is_not_built_again(self) -> None:
        same = f"Max {self.cargo_version()} (build 20261126-ab12cd3)"
        self.assertIn("already published", self.fails(FAKE_BETA_TITLE=same))
        self.assertEqual(self.ids(FAKE_BETA_TITLE=same, BETA_ALLOW_SAME_VERSION="1")["version"], self.cargo_version())

    def test_a_lower_version_than_the_published_one_is_refused(self) -> None:
        self.assertIn("already published", self.fails(FAKE_BETA_TITLE="Max 99.0.0 (build 20261126-ab12cd3)"))

    def test_no_published_release_starts_from_cargo(self) -> None:
        self.assertEqual(
            self.ids(FAKE_GH_ERROR="release not found")["version"], self.cargo_version()
        )

    def test_any_other_gh_failure_stops_instead_of_guessing(self) -> None:
        err = self.fails(FAKE_GH_ERROR="HTTP 502: Bad Gateway")
        self.assertIn("could not read the published release", err)

    def test_a_title_without_a_version_stops(self) -> None:
        self.assertIn("has no version", self.fails(FAKE_BETA_TITLE="Beta"))

    def test_no_codename_pool_and_no_timestamp_survive(self) -> None:
        for path in (SCRIPT, BETA_WORKFLOW, LOCAL):
            text = path.read_text()
            self.assertNotIn("NAMES=(", text, path)
            self.assertNotIn("CODENAME", text, path)
            self.assertNotIn("%H%M%S", text, path)
            self.assertIsNone(re.search(r"BUILD_ID=\$\(\s*date\b", text), path)

    def test_the_workflow_and_the_local_build_share_the_script(self) -> None:
        workflow = BETA_WORKFLOW.read_text()
        self.assertIn("scripts/beta_version.sh >>", workflow)
        self.assertIn("beta_version.sh", LOCAL.read_text())
        self.assertIn("HERDR_BUILD_CHANNEL: max", workflow)
        self.assertIn("HERDR_BUILD_CHANNEL=max", LOCAL.read_text())

    def test_the_binary_and_the_title_carry_the_build_id(self) -> None:
        local = LOCAL.read_text()
        self.assertIn('want="herdr-max ${VERSION} (build ${BUILD_ID})"', local)
        self.assertIn("BUILD_ID=$BUILD_ID", local)
        publish = (ROOT / "scripts" / "beta_publish.sh").read_text()
        self.assertIn('TITLE="Max ${BETA_VERSION}${BUILD_ID:+ (build ${BUILD_ID})}"', publish)

    def test_just_release_local_is_the_recipe_and_beta_local_its_alias(self) -> None:
        justfile = (ROOT / "justfile").read_text()
        self.assertIn("release-local ref=", justfile)
        self.assertIn("alias beta-local := release-local", justfile)


@unittest.skipUnless(os.name == "posix", "the beta scripts are bash")
class BetaFormulaTests(unittest.TestCase):
    def formula(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(FORMULA), *args], capture_output=True, text=True
        )

    def test_a_github_beta_serves_both_macs(self) -> None:
        out = self.formula("0.10.0", ARM, X86)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn('version "0.10.0"', out.stdout)
        self.assertIn("if Hardware::CPU.arm?", out.stdout)
        self.assertIn(f'sha256 "{ARM}"', out.stdout)
        self.assertIn(f'sha256 "{X86}"', out.stdout)
        self.assertIn("releases/download/beta/herdr-macos-x86_64", out.stdout)
        self.assertNotIn("depends_on arch", out.stdout)

    def test_a_local_beta_is_apple_silicon_only(self) -> None:
        out = self.formula("0.10.1", ARM)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("depends_on arch: :arm64", out.stdout)
        self.assertIn("releases/download/beta/herdr-macos-aarch64", out.stdout)
        self.assertNotIn("x86_64", out.stdout)
        self.assertNotIn("Hardware::CPU", out.stdout)

    def test_both_shapes_keep_the_install_and_the_test(self) -> None:
        for args in ((ARM,), (ARM, X86)):
            out = self.formula("1.0.0", *args).stdout
            self.assertTrue(out.startswith("class HerdrMax < Formula\n"))
            self.assertIn('bin.install binary => "herdr-max"', out)
            self.assertNotIn("install_symlink", out)
            self.assertNotIn("depends_on \"herdr-max\"", out)
            self.assertIn('herdr-max server live-handoff --import-exe "#{opt_bin}/herdr-max"', out)
            self.assertIn('assert_match "herdr-max #{version}"', out)
            self.assertTrue(out.endswith("end\n"))

    def test_the_alias_formula_owns_the_old_command_and_nothing_else(self) -> None:
        # Fork issue 330: herdr-max and the last herdr-beta must never both own
        # bin/herdr-beta, or the upgrade fails on a brew link conflict.
        for args in ((ARM,), (ARM, X86)):
            out = subprocess.run(
                [str(FORMULA), "--alias", "1.0.0", *args],
                capture_output=True, text=True, check=True,
            ).stdout
            maxf = self.formula("1.0.0", *args).stdout
            self.assertTrue(out.startswith("class HerdrBeta < Formula\n"))
            self.assertIn('depends_on "herdr-max"', out)
            self.assertIn("bin.mkpath", out)
            self.assertIn(
                '(bin/"herdr-beta").make_symlink Formula["herdr-max"].opt_bin/"herdr-max"',
                out,
            )
            self.assertNotIn("bin.install binary", out)
            # Same version and same download as herdr-max, so a plain
            # `brew upgrade herdr-beta` moves both together.
            self.assertIn('version "1.0.0"', out)
            self.assertIn(f'sha256 "{ARM}"', out)
            self.assertNotIn('"herdr-beta"', maxf)  # no file of that name in bin
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
        self.assertIn(
            '../scripts/beta_formula.sh --alias "${BETA_VERSION}" "${DARWIN_ARM64}" "${DARWIN_AMD64}"',
            workflow,
        )
        self.assertIn("Formula/herdr-max.rb", workflow)
        self.assertNotIn("class HerdrBeta", workflow)
        self.assertNotIn("class HerdrMax", workflow)


class BetaLinuxWorkflowTests(unittest.TestCase):
    """Linux binaries of the published beta (fork issue 239): same commit and
    version string, both static musl targets, uploaded only onto that beta."""

    def setUp(self) -> None:
        self.text = BETA_LINUX_WORKFLOW.read_text()

    def test_builds_both_static_linux_targets_on_the_beta_channel(self) -> None:
        self.assertIn("target: x86_64-unknown-linux-musl", self.text)
        self.assertIn("target: aarch64-unknown-linux-musl", self.text)
        self.assertIn("HERDR_BUILD_CHANNEL: max", self.text)
        self.assertIn("HERDR_BUILD_ID: ${{ inputs.build_id }}", self.text)
        self.assertIn("ref: ${{ inputs.ref }}", self.text)

    def test_checks_the_version_string(self) -> None:
        self.assertIn("expected 'herdr-max ${BASE} (build ${HERDR_BUILD_ID})'", self.text)

    def test_uploads_only_onto_its_own_beta_with_checksums(self) -> None:
        self.assertIn('WANT="Max ${BASE} (build ${BUILD_ID})"', self.text)
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
