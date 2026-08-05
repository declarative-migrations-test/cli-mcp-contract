#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

PROTOCOL_VERSION = "2025-03-26"
TOOLS = [
    {
        "name": "dpm_diff",
        "description": "Generate a read-only declarative migration plan.",
        "inputSchema": {
            "type": "object",
            "required": ["source", "target"],
            "properties": {
                "source": {"type": "string"},
                "target": {"type": "string"},
                "shadow": {"type": "string"},
                "format": {"enum": ["sql", "json"]},
            },
            "additionalProperties": False,
        },
    },
    {
        "name": "dpm_verify",
        "description": "Replay a plan on a shadow target and prove convergence.",
        "inputSchema": {
            "type": "object",
            "required": ["source", "target", "shadow"],
            "properties": {
                "source": {"type": "string"},
                "target": {"type": "string"},
                "shadow": {"type": "string"},
            },
            "additionalProperties": False,
        },
    },
    {
        "name": "dpm_apply",
        "description": "Apply a declarative plan only with exact target confirmation.",
        "inputSchema": {
            "type": "object",
            "required": ["source", "target", "shadow", "confirm_target"],
            "properties": {
                "source": {"type": "string"},
                "target": {"type": "string"},
                "shadow": {"type": "string"},
                "confirm_target": {"type": "string"},
                "allow_destructive": {"type": "boolean"},
            },
            "additionalProperties": False,
        },
    },
]


def response(identifier: Any, result: Any = None, error: Any = None) -> dict[str, Any]:
    payload: dict[str, Any] = {"jsonrpc": "2.0", "id": identifier}
    if error is None:
        payload["result"] = result
    else:
        payload["error"] = error
    return payload


def command_for(name: str, arguments: dict[str, Any]) -> list[str]:
    dpm = os.environ.get("DPM_BIN", "dpm")
    source = arguments.get("source")
    target = arguments.get("target")
    shadow = arguments.get("shadow")
    if not isinstance(source, str) or not isinstance(target, str):
        raise ValueError("source and target are required strings")
    base = [dpm, name.removeprefix("dpm_"), "--source", source, "--target", target]
    if shadow is not None:
        if not isinstance(shadow, str):
            raise ValueError("shadow must be a string")
        base += ["--shadow", shadow]
    if name == "dpm_diff":
        format_name = arguments.get("format", "sql")
        if format_name not in ("sql", "json"):
            raise ValueError("format must be sql or json")
        base += ["--format", format_name]
    elif name == "dpm_apply":
        if arguments.get("confirm_target") != target:
            raise PermissionError("confirm_target must exactly equal target")
        base.append("--yes")
        if arguments.get("allow_destructive") is True:
            base.append("--allow-destructive")
    return base


def call_tool(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
    if name not in {tool["name"] for tool in TOOLS}:
        raise ValueError("unknown tool")
    command = command_for(name, arguments)
    completed = subprocess.run(command, capture_output=True, text=True, timeout=300, check=False)
    return {
        "content": [{"type": "text", "text": completed.stdout}],
        "isError": completed.returncode != 0,
        "exit_code": completed.returncode,
        "stderr": completed.stderr[-8192:],
    }


def handle(message: dict[str, Any]) -> dict[str, Any] | None:
    identifier = message.get("id")
    method = message.get("method")
    if method == "initialize":
        return response(identifier, {"protocolVersion": PROTOCOL_VERSION, "capabilities": {"tools": {}}, "serverInfo": {"name": "dpm-mcp", "version": "0.1.0"}})
    if method == "notifications/initialized":
        return None
    if method == "tools/list":
        return response(identifier, {"tools": TOOLS})
    if method == "tools/call":
        params = message.get("params") or {}
        try:
            return response(identifier, call_tool(params.get("name"), params.get("arguments") or {}))
        except (ValueError, PermissionError) as error:
            return response(identifier, error={"code": -32602, "message": str(error)})
        except subprocess.TimeoutExpired:
            return response(identifier, error={"code": -32001, "message": "dpm command timed out"})
    return response(identifier, error={"code": -32601, "message": "method not found"})


def main() -> int:
    for raw in sys.stdin:
        try:
            message = json.loads(raw)
            reply = handle(message)
            if reply is not None:
                print(json.dumps(reply, separators=(",", ":")), flush=True)
        except Exception as error:
            print(json.dumps(response(None, error={"code": -32700, "message": str(error)}), separators=(",", ":")), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
