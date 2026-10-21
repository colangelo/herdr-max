"""The default build links no NATS crate (fork issue 241).

The kv task store sits behind the off-by-default `tasks-kv` cargo feature, so
upstream, Windows and Linux builds carry no NATS client. `cargo tree` with the
default features is the proof; it needs no network and builds nothing.
"""

import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
CARGO = os.environ.get("CARGO", "cargo")


def cargo(*args):
    result = subprocess.run(
        [CARGO, *args, "--locked", "--offline"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(f"cargo {' '.join(args)} failed:\n{result.stderr}")
    return result.stdout


class DefaultBuildHasNoNatsTests(unittest.TestCase):
    def test_tasks_kv_feature_exists_and_is_off_by_default(self):
        metadata = json.loads(cargo("metadata", "--no-deps", "--format-version", "1"))
        herdr = next(pkg for pkg in metadata["packages"] if pkg["name"] == "herdr")
        features = herdr["features"]
        self.assertIn("tasks-kv", features)
        self.assertNotIn("tasks-kv", features.get("default", []))

    def test_default_build_has_no_nats(self):
        tree = cargo("tree", "--edges", "normal,build", "--prefix", "none")
        names = {line.split()[0] for line in tree.splitlines() if line.strip()}
        nats = sorted(name for name in names if "nats" in name.lower())
        self.assertEqual(nats, [], f"the default build links NATS crates: {nats}")


if __name__ == "__main__":
    unittest.main()
