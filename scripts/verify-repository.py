#!/usr/bin/env python3
import json
import re
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
manifest = json.loads((root / "bootstrap-manifest.json").read_text())
expected_commit = "21eb846e356b2a5aff068b21e77903e6cca50452"
required = [
    "README.md",
    "AGENTS.md",
    "LICENSE",
    ".gitmodules",
    "bootstrap-manifest.json",
    "scripts/build-dpm.sh",
    ".github/workflows/ci.yml",
]
missing = [path for path in required if not (root / path).exists()]
if missing:
    raise SystemExit(f"missing required files: {missing}")
if manifest["production_dependency"]["commit"] != expected_commit:
    raise SystemExit("production dependency pin drifted in the manifest")

vendor = root / "vendor" / "declarative-postgres-migrate.rs"
try:
    actual_commit = subprocess.check_output(
        ["git", "-C", str(vendor), "rev-parse", "HEAD"],
        text=True,
    ).strip()
except subprocess.CalledProcessError as error:
    raise SystemExit("unable to inspect the production dependency submodule") from error
if actual_commit != expected_commit:
    raise SystemExit(
        f"production dependency checkout drifted: expected {expected_commit}, observed {actual_commit}"
    )

for path in root.rglob("*"):
    relative = path.relative_to(root)
    if (
        not path.is_file()
        or ".git" in relative.parts
        or (relative.parts and relative.parts[0] == "vendor")
        or path.stat().st_size > 1_000_000
    ):
        continue
    try:
        text = path.read_text()
    except UnicodeDecodeError:
        continue
    if any(marker in text for marker in ("<" * 7, "=" * 7, ">" * 7)):
        raise SystemExit(f"conflict marker in {relative}")
    if re.search(r"gh[pousr]_[A-Za-z0-9]{20,}|BEGIN [A-Z ]*PRIVATE KEY", text):
        raise SystemExit(f"credential-shaped content in {relative}")
print(f"validated {manifest['organization']}/{manifest['repository']}")
