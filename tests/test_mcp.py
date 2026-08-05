import importlib.util
import io
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("dpm_mcp", ROOT / "mcp" / "dpm_mcp.py")
MCP = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MCP)


class McpContractTests(unittest.TestCase):
    def test_lists_exact_guarded_tools(self):
        self.assertEqual([tool["name"] for tool in MCP.TOOLS], ["dpm_diff", "dpm_verify", "dpm_apply"])

    def test_apply_requires_exact_target_confirmation(self):
        with self.assertRaises(PermissionError):
            MCP.command_for(
                "dpm_apply",
                {"source": "source.sql", "target": "postgres://db/target", "shadow": "postgres://db/admin", "confirm_target": "postgres://db/other"},
            )

    def test_arguments_are_passed_without_shell_interpolation(self):
        target = "postgres://db/target;touch /tmp/should-not-exist"
        command = MCP.command_for(
            "dpm_diff",
            {"source": "source.sql", "target": target, "shadow": "postgres://db/admin", "format": "json"},
        )
        self.assertIn(target, command)
        self.assertNotIn("sh", command[:1])

    def test_stdio_initialize(self):
        completed = subprocess.run(
            [sys.executable, str(ROOT / "mcp" / "dpm_mcp.py")],
            input=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}) + "\n",
            capture_output=True,
            text=True,
            check=True,
        )
        reply = json.loads(completed.stdout)
        self.assertEqual(reply["result"]["protocolVersion"], MCP.PROTOCOL_VERSION)

    def test_real_diff_smoke_when_configured(self):
        dpm = os.environ.get("DPM_REAL_BIN")
        target = os.environ.get("DPM_REAL_TARGET")
        shadow = os.environ.get("DPM_REAL_SHADOW")
        if not all((dpm, target, shadow)):
            self.skipTest("real dpm smoke is not configured")
        previous = os.environ.get("DPM_BIN")
        os.environ["DPM_BIN"] = dpm
        try:
            result = MCP.call_tool(
                "dpm_diff",
                {
                    "source": str(ROOT / "fixtures" / "v2.sql"),
                    "target": target,
                    "shadow": shadow,
                    "format": "json",
                },
            )
        finally:
            if previous is None:
                os.environ.pop("DPM_BIN", None)
            else:
                os.environ["DPM_BIN"] = previous
        self.assertEqual(result["exit_code"], 0, result)
        json.loads(result["stdout"])


if __name__ == "__main__":
    unittest.main()
