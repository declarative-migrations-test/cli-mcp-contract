"""Clone the grok EnvMap product head and forbid process-env mutation in src/."""

from __future__ import annotations

import json
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
PIN = json.loads((ROOT / "env-map-product.json").read_text(encoding="utf-8"))


class EnvMapProductHead(unittest.TestCase):
    def test_src_does_not_call_set_var(self):
        work = pathlib.Path(tempfile.mkdtemp(prefix="env-map-product-"))
        try:
            subprocess.run(["git", "init", str(work)], check=True, capture_output=True, text=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(work),
                    "remote",
                    "add",
                    "origin",
                    f"https://github.com/{PIN['repository']}.git",
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            fetched = subprocess.run(
                ["git", "-C", str(work), "fetch", "--depth", "1", "origin", PIN["sha"]],
                capture_output=True,
                text=True,
            )
            self.assertEqual(fetched.returncode, 0, fetched.stderr)
            subprocess.run(
                ["git", "-C", str(work), "checkout", "--detach", "FETCH_HEAD"],
                check=True,
                capture_output=True,
                text=True,
            )
            sha = subprocess.check_output(
                ["git", "-C", str(work), "rev-parse", "HEAD"], text=True
            ).strip()
            self.assertEqual(sha, PIN["sha"])
            for needle in PIN["forbidden"]:
                grep = subprocess.run(
                    ["git", "-C", str(work), "grep", "-n", needle, "--", *PIN.get("paths", ["src"])],
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(grep.stdout.strip(), "", f"forbidden {needle}:\n{grep.stdout}")
        finally:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
