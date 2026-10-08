from __future__ import annotations

import re
import unittest
from pathlib import Path

BETA_WORKFLOW = (
    Path(__file__).resolve().parent.parent / ".github" / "workflows" / "beta.yml"
)


class BetaBuildIdTests(unittest.TestCase):
    """Guard the beta build-id scheme: a monotonic run number plus a Juventus
    surname codename (e.g. ``0.7.5-ac-beta.45-zakaria``).

    The build id must lead with the workflow run number so Homebrew keeps
    ``brew upgrade herdr-beta`` monotonic. A retired scheme stamped a raw UTC
    timestamp; this suite fails if that (or any ``date``-derived id) comes back,
    or if the codename pool disappears. Rationale + the "do not revert" note live
    in ``.claude/skills/herdr-release/SKILL.md``.
    """

    def setUp(self) -> None:
        self.assertTrue(BETA_WORKFLOW.exists(), f"missing {BETA_WORKFLOW}")
        self.text = BETA_WORKFLOW.read_text()

    def test_build_id_leads_with_the_run_number(self) -> None:
        self.assertIn("RUN_NUMBER: ${{ github.run_number }}", self.text)
        self.assertIn('BUILD_ID="${RUN_NUMBER}-${NAME}"', self.text)

    def test_build_id_is_not_derived_from_a_timestamp(self) -> None:
        # The retired `date -u +%Y%m%d%H%M%S` form is exactly what the run-number
        # scheme replaces; a date-derived id orders only by wall clock and reads
        # as an opaque blob. Fail loudly if it returns.
        self.assertIsNone(
            re.search(r"BUILD_ID=\$\(\s*date\b", self.text),
            "beta build id must not be derived from `date`; lead with the run number",
        )
        self.assertNotIn("%Y%m%d", self.text)

    def test_codename_pool_is_present_and_valid(self) -> None:
        match = re.search(r"NAMES=\(([^)]*)\)", self.text)
        assert match is not None, "beta.yml must define a NAMES=(...) codename pool"
        names = match.group(1).split()
        self.assertGreaterEqual(
            len(names), 20, "codename pool should keep a healthy spread of surnames"
        )
        invalid = [n for n in names if not re.fullmatch(r"[a-z]+", n)]
        self.assertEqual(
            invalid,
            [],
            "codenames must be lowercase ascii so they are valid version tokens",
        )

    def test_version_string_combines_base_channel_and_build_id(self) -> None:
        self.assertIn("version=${BASE}-ac-beta.${BUILD_ID}", self.text)

    def test_codename_can_be_pinned_and_is_validated(self) -> None:
        # Pinning keeps a run of builds reading the same while the run number
        # still increments, so Homebrew ordering is untouched. An unchecked pin
        # would let a typo mint a version token nothing else in the channel
        # shares, so the workflow must reject a name outside the pool.
        self.assertIn("CODENAME: ${{ inputs.codename }}", self.text)
        self.assertIn('if [ -n "${CODENAME}" ]; then', self.text)
        self.assertIn('*" ${CODENAME} "*) NAME="${CODENAME}" ;;', self.text)
        self.assertIn("is not in the pool", self.text)


if __name__ == "__main__":
    unittest.main()
